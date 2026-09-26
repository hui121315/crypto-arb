"""Isolated cache policy tests; never build or clean the product workspace."""

from contextlib import redirect_stdout
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import tomllib
import unittest
from unittest.mock import patch

import cache_hygiene as cache


class CacheTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="crossline-cache-test-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name).resolve()
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        self.target = self.root / "target"
        self.write("target/CACHEDIR.TAG", cache.CACHE_TAG + "\n")
        self.write(".env", "fixture-only")
        self.write("data/orders.sqlite", "fixture-only")
        self.write(".crossline-runtime/data/journal", "fixture-only")
        self.policy = cache.Policy(soft=64 * 1024, hard=256 * 1024, min_free=0)
        self.cleaner = cache.Cleaner(self.root, self.policy)
        self.idle_patch = patch.object(cache, "active_processes", return_value=[])
        self.idle_patch.start()
        self.addCleanup(self.idle_patch.stop)
        self.output = io.StringIO()
        self.redirect = redirect_stdout(self.output)
        self.redirect.__enter__()
        self.addCleanup(self.redirect.__exit__, None, None, None)

    def write(self, relative, value):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(value)
        return path

    def incremental(self, name, old=False, frontend=False):
        base = "frontend/target/wasm32-unknown-unknown" if frontend else "target"
        if frontend:
            self.write("frontend/target/CACHEDIR.TAG", cache.CACHE_TAG + "\n")
        path = self.write(f"{base}/debug/incremental/{name}/session/state", "x" * 40000)
        entry = path.parent.parent
        if old:
            stamp = time.time() - 9 * 86400
            for item in [path, path.parent, entry]:
                os.utime(item, (stamp, stamp))
        return entry

    def assert_protected(self):
        for path in (".env", "data/orders.sqlite", ".crossline-runtime/data/journal"):
            self.assertEqual((self.root / path).read_text(), "fixture-only")

    def test_below_limit_keeps_old_cache(self):
        old = self.incremental("old", old=True)
        self.cleaner.maintain("--auto-clean")
        self.assertTrue(old.exists())
        self.assert_protected()

    def test_soft_limit_only_removes_old_incrementals_across_both_workspaces(self):
        old = self.incremental("old", old=True, frontend=True)
        fresh = self.incremental("fresh")
        self.cleaner.maintain("--auto-clean")
        self.assertFalse(old.exists())
        self.assertTrue(fresh.exists())
        self.assert_protected()

    def test_recent_child_keeps_old_parent(self):
        old = self.incremental("old", old=True)
        (old / "session/state").touch()
        self.incremental("fresh")
        self.cleaner.maintain("--auto-clean")
        self.assertTrue(old.exists())

    def test_trim_keeps_dependencies_and_executable(self):
        fresh = self.incremental("fresh")
        dep = self.write("target/debug/deps/dep.rlib", "keep")
        binary = self.write("target/debug/crypto-arb-api", "keep")
        self.cleaner.maintain("--clean-incremental")
        self.assertFalse(fresh.exists())
        self.assertTrue(dep.exists() and binary.exists())
        self.assert_protected()

    def test_dry_run_does_not_delete(self):
        old = self.incremental("old", old=True)
        fresh = self.incremental("fresh")
        self.cleaner.dry_run = True
        self.cleaner.maintain("--auto-clean")
        self.assertTrue(old.exists() and fresh.exists())
        self.assertIn("Would remove", self.output.getvalue())

    def test_busy_and_unavailable_process_state_fail_closed(self):
        entry = self.incremental("fresh")
        with patch.object(cache, "active_processes", return_value=["123 cargo"]):
            with self.assertRaises(cache.UnsafeCache):
                self.cleaner.maintain("--clean-project")
        with patch.object(cache, "active_processes", side_effect=OSError("denied")):
            with self.assertRaises(OSError):
                self.cleaner.maintain("--clean-project")
        self.assertTrue(entry.exists())

    def test_process_started_after_initial_check_is_detected(self):
        entry = self.incremental("fresh")
        with patch.object(cache, "active_processes", side_effect=[[], ["123 rustc"]]):
            with self.assertRaises(cache.UnsafeCache):
                self.cleaner.maintain("--clean-incremental")
        self.assertTrue(entry.exists())

    def test_missing_cargo_tag_and_tracked_files_are_rejected(self):
        tag = self.target / "CACHEDIR.TAG"
        tag.unlink()
        with self.assertRaises(cache.UnsafeCache):
            self.cleaner.maintain("--clean-project")
        self.write("target/CACHEDIR.TAG", cache.CACHE_TAG)
        keep = self.write("target/important", "keep")
        subprocess.run(["git", "-C", str(self.root), "add", "target/important"], check=True)
        with self.assertRaises(cache.UnsafeCache):
            self.cleaner.maintain("--clean-project")
        self.assertTrue(keep.exists())

    def test_symlinked_target_debug_and_lock_are_rejected(self):
        outside = self.root / "outside"
        outside.mkdir()
        debug = self.target / "debug"
        debug.symlink_to(outside, target_is_directory=True)
        with self.assertRaises(cache.UnsafeCache):
            self.cleaner.maintain("--clean-incremental")
        debug.unlink()
        lock = self.root / ".crossline-runtime/cache-hygiene/maintenance.lock"
        lock.unlink()
        lock.symlink_to(self.root / ".env")
        with self.assertRaises(cache.UnsafeCache):
            self.cleaner.maintain("--clean-project")
        self.assert_protected()

    def test_only_one_cleaner_can_hold_lock(self):
        with self.cleaner.lock():
            with self.assertRaises(cache.UnsafeCache):
                cache.Cleaner(self.root).maintain("--clean-project")

    def test_symlinked_target_is_not_followed(self):
        original = self.root / "original-target"
        self.target.rename(original)
        self.target.symlink_to(original, target_is_directory=True)
        with self.assertRaises(cache.UnsafeCache):
            self.cleaner.maintain("--clean-project")
        self.assertTrue((original / "CACHEDIR.TAG").exists())

    def test_ephemeral_cleanup_is_project_only(self):
        for relative in ("frontend/dist/index.html", "frontend/styles/.generated/input.css", ".trunk/tmp"):
            self.write(relative, "generated")
        dependency = self.write("node_modules/keep", "keep")
        screenshot = self.write("output/design.png", "keep")
        self.cleaner.maintain("--clean-ephemeral")
        self.assertFalse((self.root / "frontend/dist").exists())
        self.assertTrue(dependency.exists() and screenshot.exists())
        self.assert_protected()

    def test_low_disk_pressure_trims_fresh_incrementals(self):
        entries = [self.incremental(name) for name in ("a", "b", "c")]
        self.cleaner.policy = cache.Policy(soft=64000, hard=1000000, min_free=10**30)
        self.cleaner.maintain("--auto-clean")
        self.assertTrue(all(not path.exists() for path in entries))

    def test_release_only_above_limit_is_reported_not_removed(self):
        release = self.write("target/release/product", "x" * 300000)
        self.cleaner.maintain("--auto-clean")
        self.assertTrue(release.exists())
        self.assertIn("still above hard limit", self.output.getvalue())

    def test_process_parser_catches_absolute_paths_without_args(self):
        self.idle_patch.stop()
        processes = "1 /usr/bin/cargo\n2 /some/target/debug/deps/test-abc\n3 /opt/trunk\n4 /usr/bin/python3\n"
        with patch.object(cache, "run", return_value=subprocess.CompletedProcess([], 0, processes)):
            self.assertEqual(cache.active_processes(), ["1 cargo", "2 test-abc", "3 trunk"])

    def test_invalid_configuration(self):
        with patch.dict(os.environ, {"PROJECT_CLEAN_HARD_LIMIT_GIB": "0"}):
            with self.assertRaises(ValueError):
                cache.Policy.from_env()

    def test_real_cargo_build_running_binary_guard_and_debug_cleanup(self):
        self.write("Cargo.toml", '[package]\nname="cache-probe"\nversion="0.0.0"\nedition="2021"\n[workspace]\n')
        self.write("src/main.rs", 'fn main() { std::thread::sleep(std::time::Duration::from_secs(60)); }')
        env = {k: v for k, v in os.environ.items() if not k.startswith("CARGO_TARGET_")}
        subprocess.run(["cargo", "build", "--offline", "--quiet", "--manifest-path",
                        str(self.root / "Cargo.toml"), "--target-dir", str(self.target)],
                       cwd=self.root, env=env, check=True, timeout=60)
        subprocess.run(["cargo", "build", "--offline", "--quiet", "--target", "wasm32-unknown-unknown",
                        "--manifest-path", str(self.root / "Cargo.toml"), "--target-dir", str(self.target)],
                       cwd=self.root, env=env, check=True, timeout=60)
        binary = self.target / "debug/cache-probe"
        self.idle_patch.stop()
        process = subprocess.Popen([str(binary)])
        try:
            with self.assertRaises(cache.UnsafeCache):
                self.cleaner.maintain("--clean-project")
            self.assertTrue(binary.exists())
        finally:
            process.terminate()
            process.wait(timeout=5)
        self.idle_patch.start()
        release = self.write("target/release/keep", "release")
        external = self.write("external-cache/debug/keep", "external")
        wasm = self.target / "wasm32-unknown-unknown/debug/cache-probe.wasm"
        self.assertTrue(wasm.exists())
        source = (self.root / "src/main.rs").read_bytes()
        lock = (self.root / "Cargo.lock").read_bytes()
        with patch.dict(os.environ, {"CARGO_BUILD_BUILD_DIR": str(external.parent.parent)}):
            self.cleaner.maintain("--auto-clean")
        self.assertFalse(binary.exists())
        self.assertFalse(wasm.exists(), self.output.getvalue())
        self.assertTrue(release.exists())
        self.assertTrue(external.exists())
        self.assertEqual((self.root / "src/main.rs").read_bytes(), source)
        self.assertEqual((self.root / "Cargo.lock").read_bytes(), lock)
        self.cleaner.maintain("--clean-project")
        self.assertFalse(release.exists())
        self.assert_protected()

    def test_project_entrypoints_and_profiles(self):
        root = Path(__file__).resolve().parent.parent
        for relative in ("scripts/dev_up.sh", "scripts/dev_down.sh", "scripts/finish_batch.sh"):
            source = (root / relative).read_text()
            self.assertIn('scripts/cache_hygiene.sh" --auto-clean', source)
            subprocess.run(["bash", "-n", str(root / relative)], check=True)
        for relative in ("Cargo.toml", "frontend/Cargo.toml"):
            profiles = tomllib.loads((root / relative).read_text())["profile"]
            self.assertIs(profiles["test"]["incremental"], False)
            self.assertIs(profiles["dev"].get("incremental", True), True)
        scripts = json.loads((root / "package.json").read_text())["scripts"]
        self.assertIn("--dry-run", scripts["cache:preview"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
