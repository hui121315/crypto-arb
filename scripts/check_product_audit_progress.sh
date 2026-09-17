#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
HIST="$ROOT/docs/audit_history/PRODUCT_AUDIT_HISTORY.md"
MODE="${1:-check}"

if [ "$MODE" != "check" ] && [ "$MODE" != "--goal" ]; then
  printf 'usage: %s [--goal]\n' "$0" >&2
  exit 2
fi

if [ ! -s "$DOC" ]; then
  printf 'product audit progress gate failed: missing or empty %s\n' "${DOC#$ROOT/}" >&2
  exit 1
fi

python3 - "$DOC" "$HIST" "$MODE" <<'PY'
from decimal import Decimal, ROUND_HALF_UP
from pathlib import Path
import re
import sys

mode = sys.argv[3] if len(sys.argv) > 3 else "check"
if mode not in {"check", "--goal"}:
    raise SystemExit("usage: check_product_audit_progress.sh [--goal]")

doc = Path(sys.argv[1])
hist = Path(sys.argv[2])
text = doc.read_text(encoding="utf-8")
hist_text = hist.read_text(encoding="utf-8") if hist.exists() else ""
markers = ("✅", "🟡", "⏳", "➖", "❌")

def marker_counts(source_text):
    counts = {marker: 0 for marker in markers}
    for line in source_text.splitlines():
        if not line.startswith("|"):
            continue
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if len(cells) < 2:
            continue
        for cell in cells[:2]:
            for marker in markers:
                counts[marker] += cell.count(marker)
    return counts


main_actual = marker_counts(text)
history_actual = marker_counts(hist_text)
actual = {
    marker: main_actual[marker] + history_actual[marker] for marker in markers
}

total = sum(actual.values())
if total == 0:
    raise SystemExit("product audit progress gate failed: no status markers found")

def progress(counts):
    count_total = sum(counts.values())
    if count_total == 0:
        return Decimal("0.00")
    return (
        (Decimal(counts["✅"]) + Decimal(counts["🟡"]) * Decimal("0.5"))
        * Decimal(100)
        / Decimal(count_total)
    ).quantize(Decimal("0.01"), rounding=ROUND_HALF_UP)

computed = progress(actual)
main_computed = progress(main_actual)
history_computed = progress(history_actual)

def section_between(text, start_pattern, end_pattern):
    start = re.search(start_pattern, text, re.MULTILINE)
    if start is None:
        raise SystemExit("product audit progress gate failed: missing active roadmap section")
    tail = text[start.end():]
    end = re.search(end_pattern, tail, re.MULTILINE)
    return tail[: end.start()] if end else tail

overview_section = section_between(text, r"^##\s+0\.", r"^##\s+1\.")
expected_overview_counts = {
    "✅": 2,
    "🟡": 2,
    "⏳": 0,
    "➖": 0,
    "❌": 0,
}
if marker_counts(overview_section) != expected_overview_counts:
    raise SystemExit(
        "product audit progress gate failed: section 0 compact overview drifted"
    )
overview_partial_domains = set()
for line in overview_section.splitlines():
    if not line.startswith("|"):
        continue
    cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
    if len(cells) >= 2 and "🟡 部分完成" in cells[1]:
        overview_partial_domains.add(cells[0])
if overview_partial_domains != {
    "PR-M 真实下单验收",
    "PR-FR 真实运行态验收",
}:
    raise SystemExit(
        "product audit progress gate failed: section 0 may only retain "
        "PR-M/PR-FR external acceptance"
    )
if "| `AUD-" in overview_section:
    raise SystemExit(
        "product audit progress gate failed: detailed AUD rows leaked into section 0"
    )
if "附录 H" not in overview_section or "## 附录 H：§0 旧详细总览与 AUD 状态表" not in hist_text:
    raise SystemExit(
        "product audit progress gate failed: missing archived detailed section 0"
    )

current_gap_section = section_between(
    text,
    r"^###\s+.*6\.2\s+当前 P0 Bug / 业务缺口清单\s*$",
    r"^###\s+.*6\.3\s+建议整改顺序\s*$",
)
if "附录 F" not in current_gap_section:
    raise SystemExit(
        "product audit progress gate failed: section 6.2 must point to archived legacy facts"
    )

current_gap_counts = marker_counts(current_gap_section)
expected_current_gap_counts = {
    "✅": 2,
    "🟡": 2,
    "⏳": 0,
    "➖": 0,
    "❌": 0,
}
if current_gap_counts != expected_current_gap_counts:
    raise SystemExit(
        "product audit progress gate failed: section 6.2 current-fact matrix drifted\n"
        + "\n".join(
            f"  {marker} expected={expected_current_gap_counts[marker]} "
            f"actual={current_gap_counts[marker]}"
            for marker in markers
        )
    )

current_partial_domains = set()
for line in current_gap_section.splitlines():
    if not line.startswith("|"):
        continue
    cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
    if len(cells) >= 2 and "🟡 部分完成" in cells[1]:
        current_partial_domains.add(cells[0])
expected_partial_domains = {"PR-M 真实下单验收", "PR-FR 真实运行态验收"}
if current_partial_domains != expected_partial_domains:
    raise SystemExit(
        "product audit progress gate failed: section 6.2 may only retain "
        "PR-M/PR-FR external acceptance\n  actual="
        + ", ".join(sorted(current_partial_domains))
    )

next_audit_section = section_between(
    text,
    r"^###\s+.*6\.4\s+下一批继续审计重点\s*$",
    r"^###\s+.*6\.5\s+下一步执行队列\s*$",
)
if marker_counts(next_audit_section) != {marker: 0 for marker in markers}:
    raise SystemExit(
        "product audit progress gate failed: section 6.4 must not duplicate the 6.5 queue"
    )
if "不再维护第二套状态队列" not in next_audit_section:
    raise SystemExit(
        "product audit progress gate failed: section 6.4 missing single-queue policy"
    )
if "## 附录 F：§6.1 / §6.2 / §6.4 旧当前结论快照" not in hist_text:
    raise SystemExit(
        "product audit progress gate failed: missing archived legacy current-facts appendix"
    )

governance_section = section_between(
    text,
    r"^##\s+1\.\s+审计原则",
    r"^##\s+5\.\s+当前初步观察",
)
expected_governance_counts = {
    "✅": 4,
    "🟡": 0,
    "⏳": 0,
    "➖": 0,
    "❌": 0,
}
if marker_counts(governance_section) != expected_governance_counts:
    raise SystemExit(
        "product audit progress gate failed: sections 1-4 compact contracts drifted"
    )
if "第一批文件清单" in governance_section or "本轮官方文档抽样核验" in governance_section:
    raise SystemExit(
        "product audit progress gate failed: archived sections 1-4 leaked into current facts"
    )
if "## 附录 G：§1-§4 旧审计入口快照" not in hist_text:
    raise SystemExit(
        "product audit progress gate failed: missing archived sections 1-4 appendix"
    )

roadmap_preface = section_between(
    text,
    r"^##\s+6\.\s+当前阶段结论与整改路线图",
    r"^###\s+.*6\.1\s+产品可行性判断",
)
if "2026-06-03" in roadmap_preface or "53 条 `EndpointSpec`" in roadmap_preface:
    raise SystemExit(
        "product audit progress gate failed: stale PR-FW preface leaked into current facts"
    )
if "仅 PR-M 与 PR-FR" not in roadmap_preface:
    raise SystemExit(
        "product audit progress gate failed: section 6 preface lost external-only boundary"
    )
if "## 附录 I：§6 旧阶段性前言快照" not in hist_text:
    raise SystemExit(
        "product audit progress gate failed: missing archived section 6 preface"
    )

active = {marker: 0 for marker in markers}
active_section = section_between(
    text,
    r"^###\s+.*6\.3\s+建议整改顺序\s*$",
    r"^###\s+.*6\.4\s+",
)

live_acceptance_rows = {
    "PR-M Settings/API Status UX": (
        "scripts/check_live_order_runtime_acceptance.py",
        "live-sample-acceptance:<venue>",
    ),
    "PR-FR API Status Evidence Matrix, Venue Runtime Health & Settings Diagnostics Contract": (
        "scripts/check_private_order_stream_live_sample_acceptance.py",
        "private-order-stream-live-sample-acceptance:<venue>",
    ),
}
for label, required_markers in live_acceptance_rows.items():
    prefix = f"| `{label}` |"
    rows = [line for line in active_section.splitlines() if line.startswith(prefix)]
    if len(rows) != 1:
        raise SystemExit(
            "product audit progress gate failed: expected one current live acceptance "
            f"row for {label}, got {len(rows)}"
        )
    row = rows[0]
    if len(row) > 1_600:
        raise SystemExit(
            "product audit progress gate failed: live acceptance current row regressed "
            f"to a historical narrative ({label}, {len(row)} chars)"
        )
    if "附录 J" not in row or any(marker not in row for marker in required_markers):
        raise SystemExit(
            "product audit progress gate failed: live acceptance current row lost its "
            f"archive or verifier contract ({label})"
        )

appendix_j = "## 附录 J：PR-M / PR-FR live acceptance 逐批本地闭环快照"
if appendix_j not in hist_text:
    raise SystemExit(
        "product audit progress gate failed: missing archived PR-M/PR-FR history"
    )
archived_live_markers = {
    "PR-M": "live-sample-self-test-fixture-reuse-gate",
    "PR-FR": "settings-selected-venue-trading-runtime-all-ok-local-capture-shaped-browser-gate",
}
for label, marker in archived_live_markers.items():
    if marker not in hist_text:
        raise SystemExit(
            "product audit progress gate failed: archived live acceptance history lost "
            f"the {label} continuity marker"
        )
    if marker in active_section:
        raise SystemExit(
            "product audit progress gate failed: archived live acceptance narrative "
            f"leaked back into the current {label} row"
        )

for line in active_section.splitlines():
    if not line.startswith("| `PR-"):
        continue
    cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
    if len(cells) < 2:
        continue
    for marker in markers:
        active[marker] += cells[1].count(marker)

active_total = sum(active.values())
active_text = "active PR roadmap unavailable"
if active_total:
    active_computed = progress(active)
    active_text = (
        "active PR roadmap "
        f"({', '.join(f'{marker}{active[marker]}' for marker in markers)}; "
        f"{active_computed}%)"
    )

main_text = (
    "current main audit "
    f"({', '.join(f'{marker}{main_actual[marker]}' for marker in markers)}; "
    f"{main_computed}%)"
)
history_text = (
    "history archive "
    f"({', '.join(f'{marker}{history_actual[marker]}' for marker in markers)}; "
    f"{history_computed}%)"
)
global_text = (
    "combined audit continuity "
    f"({', '.join(f'{marker}{actual[marker]}' for marker in markers)}; {computed}%)"
)
if mode == "--goal":
    print(f"OK product audit goal progress {active_text}")
else:
    print(
        "OK product audit progress gate "
        f"{active_text}; {main_text}; {history_text}; {global_text}"
    )
PY
