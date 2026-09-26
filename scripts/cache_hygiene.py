"""Bounded, project-only Cargo cache maintenance at idle checkpoints."""

import argparse
from contextlib import contextmanager
from dataclasses import dataclass
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

GIB = 1024 ** 3
CACHE_TAG = "Signature: 8a477f597d28d172789f06886806bc55"


class UnsafeCache(RuntimeError):
    pass


@dataclass(frozen=True)
class Policy:
    soft: int = 16 * GIB
    hard: int = 32 * GIB
    min_free: int = 20 * GIB
    stale_days: int = 7

    @classmethod
    def from_env(cls):
        soft = int(os.getenv("PROJECT_CLEAN_THRESHOLD_GIB", "16"))
        hard = int(os.getenv("PROJECT_CLEAN_HARD_LIMIT_GIB", "32"))
        free = int(os.getenv("PROJECT_CLEAN_MIN_FREE_GIB", "20"))
        days = int(os.getenv("PROJECT_CACHE_STALE_DAYS", "7"))
        if not 0 < soft <= hard or free < 0 or days < 1:
            raise ValueError("Require 0 < soft <= hard, min_free >= 0, stale_days >= 1")
        return cls(soft * GIB, hard * GIB, free * GIB, days)


def run(*args, **kwargs):
    return subprocess.run(args, check=True, capture_output=True, text=True, timeout=30, **kwargs)


def active_processes():
    # comm, not args: avoids leaking credentials and catches absolute executables.
    output = run("ps", "-axo", "pid=,comm=").stdout
    busy = []
    for line in output.splitlines():
        parts = line.strip().split(None, 1)
        if len(parts) != 2:
            continue
        pid, command = parts
        name = Path(command).name
        if (name in {"cargo", "rustc", "trunk", "wasm-bindgen", "wasm-opt",
                     "cargo-clippy", "clippy-driver", "rust-lld", "crypto-arb-api"}
                or "/target/debug/" in command or "/target/release/" in command):
            busy.append(f"{pid} {name}")
    return busy


class Cleaner:
    def __init__(self, root, policy=None, dry_run=False):
        self.root = Path(root).resolve()
        self.policy = policy or Policy.from_env()
        self.dry_run = dry_run
        self.targets = [self.root / "target", self.root / "frontend/target"]

    def safe_path(self, path):
        path = Path(path)
        relative = path.relative_to(self.root)
        current = self.root
        for part in relative.parts:
            current /= part
            if current.is_symlink():
                raise UnsafeCache(f"Refusing symlink: {current}")
        if path.resolve() != path:
            raise UnsafeCache(f"Path escapes project: {path}")
        return path

    def untracked(self, path):
        tracked = run("git", "-C", str(self.root), "ls-files", "-z", "--",
                      str(path.relative_to(self.root))).stdout
        if tracked:
            raise UnsafeCache(f"Refusing Git-tracked artifacts: {path}")

    def validate_target(self, target):
        self.safe_path(target)
        if not target.exists():
            return
        tag = self.safe_path(target / "CACHEDIR.TAG")
        if not tag.is_file() or not tag.read_text().startswith(CACHE_TAG):
            raise UnsafeCache(f"Missing Cargo CACHEDIR.TAG: {target}")
        self.untracked(target)

    def size(self, path):
        self.safe_path(path)
        if not path.exists():
            return 0
        # Allocated blocks, not sparse/logical file length; do not scan $HOME.
        return int(run("du", "-sk", str(path)).stdout.split()[0]) * 1024

    def total(self):
        return sum(self.size(target) for target in self.targets)

    def idle(self):
        processes = active_processes()
        if processes:
            raise UnsafeCache("Build/service active; deferred: " + ", ".join(processes))

    @contextmanager
    def lock(self):
        state = self.safe_path(self.root / ".crossline-runtime/cache-hygiene")
        state.mkdir(parents=True, exist_ok=True)
        path = self.safe_path(state / "maintenance.lock")
        descriptor = os.open(path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
        try:
            try:
                fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError as error:
                raise UnsafeCache("Another cache maintenance is active; deferred") from error
            yield
        finally:
            os.close(descriptor)

    def debug_dirs(self, target):
        if not target.exists():
            return []
        paths = [target / "debug"]
        # Cargo's native and cross-target layouts, e.g. wasm32-unknown-unknown/debug.
        paths.extend(child / "debug" for child in target.iterdir()
                     if child.name != "debug" and child.is_dir())
        return [self.safe_path(path) for path in paths if path.exists()]

    def incrementals(self, stale_only):
        cutoff = time.time() - self.policy.stale_days * 86400
        entries = []
        for target in self.targets:
            for debug in self.debug_dirs(target):
                incremental = self.safe_path(debug / "incremental")
                if not incremental.is_dir():
                    continue
                for entry in incremental.iterdir():
                    self.safe_path(entry)
                    if not entry.is_dir():
                        continue
                    # Session children change independently of their parent directory.
                    latest = entry.stat().st_mtime
                    for base, dirs, files in os.walk(entry, followlinks=False):
                        for name in dirs + files:
                            child = Path(base) / name
                            if not child.is_symlink():
                                latest = max(latest, child.stat().st_mtime)
                    if not stale_only or latest < cutoff:
                        entries.append(entry)
        return entries

    def remove_incrementals(self, stale_only):
        for entry in self.incrementals(stale_only):
            self.safe_path(entry)
            self.untracked(entry)
            self.idle()
            print(f"{'Would remove' if self.dry_run else 'Remove'} incremental: {entry.relative_to(self.root)}")
            if not self.dry_run:
                shutil.rmtree(entry)

    def cargo_clean(self, target, full=False):
        self.validate_target(target)
        if not target.exists():
            return
        debug_dirs = self.debug_dirs(target)
        if not full and not debug_dirs:
            return
        manifest = target.parent / "Cargo.toml"
        commands = []
        if full:
            commands.append((target, []))
        else:
            for debug in debug_dirs:
                # Anchor each layout explicitly: Cargo 1.95's profile-scoped
                # cleanup can leave cross-target artifacts with --target alone.
                commands.append((debug.parent, ["--profile", "dev"]))
        for artifact_root, flags in commands:
            self.safe_path(artifact_root)
            self.idle()
            command = ["cargo", "clean", "--frozen", "--manifest-path", str(manifest),
                       "--target-dir", str(artifact_root), "--config",
                       "build.build-dir=" + json.dumps(str(artifact_root)), *flags]
            print(f"{'Would run' if self.dry_run else 'Run'}: {' '.join(command)}", flush=True)
            if not self.dry_run:
                subprocess.run(command, cwd=self.root, check=True, timeout=120)

    def ephemeral(self):
        for relative in ("frontend/dist", ".trunk", "frontend/styles/.generated"):
            path = self.safe_path(self.root / relative)
            self.untracked(path)
            if path.is_dir():
                self.idle()
                print(f"{'Would remove' if self.dry_run else 'Remove'} generated: {relative}")
                if not self.dry_run:
                    shutil.rmtree(path)

    def report(self):
        for target in self.targets:
            print(f"{target.relative_to(self.root)}: {self.size(target) / GIB:.2f} GiB")
        print(f"Policy: soft={self.policy.soft / GIB:g} GiB, hard={self.policy.hard / GIB:g} GiB, "
              f"min_free={self.policy.min_free / GIB:g} GiB, stale={self.policy.stale_days} days")
        print("Protected: source, .env, data, runtime state, Git, global caches, Trash; "
              "auto-clean also preserves release/custom profiles and frontend/dist.")
        if any(os.getenv(key) for key in ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR", "CARGO_BUILD_BUILD_DIR")):
            print("Note: custom Cargo target/build directories are not managed by this project cleaner.")
        if os.getenv("CARGO_INCREMENTAL") or os.getenv("CARGO_BUILD_INCREMENTAL"):
            print("Note: an incremental environment override can supersede the test profile policy.")

    def maintain(self, mode):
        with self.lock():
            self.idle()
            for target in self.targets:
                self.validate_target(target)
            before = self.total()
            if mode == "--clean-project":
                for target in self.targets:
                    self.cargo_clean(target, full=True)
                self.ephemeral()
            elif mode == "--clean-ephemeral":
                self.ephemeral()
            elif mode == "--clean-incremental":
                self.remove_incrementals(stale_only=False)
            else:
                if before < self.policy.soft:
                    print(f"Retain warm cache: {before / GIB:.2f} GiB (below soft limit)")
                    return
                self.remove_incrementals(stale_only=True)
                total = self.total()
                pressure = shutil.disk_usage(self.root).free < self.policy.min_free
                if total >= self.policy.hard or (total >= self.policy.soft and pressure):
                    self.remove_incrementals(stale_only=False)
                    # Only debug/test artifacts; preserve distributable release builds.
                    for target in sorted(self.targets, key=self.size, reverse=True):
                        if self.total() < self.policy.soft:
                            break
                        self.cargo_clean(target)
                if self.total() >= self.policy.hard:
                    print("WARNING: still above hard limit (or dry-run); release/custom artifacts "
                          "are retained. Inspect cache:report; cache:clean is explicit full cleanup.")
            after = self.total()
            print(f"Cargo cache: {before / GIB:.2f} -> {after / GIB:.2f} GiB; "
                  f"released {max(0, before - after) / GIB:.2f} GiB")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_mutually_exclusive_group()
    for mode in ("--report", "--auto-clean", "--clean-incremental", "--clean-project", "--clean-ephemeral"):
        modes.add_argument(mode, dest="mode", action="store_const", const=mode)
    parser.set_defaults(mode="--report")
    parser.add_argument("--dry-run", action="store_true", help="Print actions without deleting artifacts")
    args = parser.parse_args()
    try:
        cleaner = Cleaner(Path(__file__).resolve().parent.parent, dry_run=args.dry_run)
        if args.mode == "--report":
            cleaner.report()
        else:
            cleaner.maintain(args.mode)
    except (UnsafeCache, OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"Cache maintenance skipped/failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
