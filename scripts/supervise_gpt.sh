#!/usr/bin/env bash
# 每 10 分钟由 launchd / loop 调起，向 docs/SUPERVISION_LOG_AUTO.md 追加一段事实快照。
# 故意不调用 cargo，避免与 GPT 抢 target lock。
set -u

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LOG="$REPO_ROOT/docs/SUPERVISION_LOG_AUTO.md"
BOARD="$REPO_ROOT/docs/PRODUCT_PLAN_EXECUTION.md"

ts="$(date '+%Y-%m-%d %H:%M:%S %Z')"
head_hash="$(git -C "$REPO_ROOT" rev-parse --short HEAD 2>/dev/null || echo '?')"
head_msg="$(git -C "$REPO_ROOT" log -1 --pretty=%s 2>/dev/null || echo '?')"
branch="$(git -C "$REPO_ROOT" branch --show-current 2>/dev/null || echo '?')"

status_short="$(git -C "$REPO_ROOT" status -u --short 2>/dev/null)"
recent_log="$(git -C "$REPO_ROOT" log -n 5 --oneline 2>/dev/null)"

if [ -f "$BOARD" ]; then
  done_cnt="$(grep -c -E '^\s*-\s+\[x\]' "$BOARD" 2>/dev/null || echo 0)"
  todo_cnt="$(grep -c -E '^\s*-\s+\[ \]' "$BOARD" 2>/dev/null || echo 0)"
  current="$(awk '/^## 当前正在执行/{flag=1; next} /^## /{flag=0} flag' "$BOARD" 2>/dev/null | tr -s '\n' ' ' | sed -E 's/^ +//;s/ +$//')"
else
  done_cnt='?'
  todo_cnt='?'
  current='(看板缺失)'
fi

dirty_files="$(echo "$status_short" | wc -l | awk '{print $1}')"
if [ -z "$status_short" ]; then
  dirty_files=0
fi

{
  echo ""
  echo "## AUTO · $ts"
  echo ""
  echo "- HEAD: \`$head_hash\` $head_msg"
  echo "- 分支：\`$branch\`"
  echo "- 看板勾选：[x] $done_cnt · [ ] $todo_cnt"
  echo "- 工作树文件数：$dirty_files"
  if [ -n "$current" ]; then
    echo "- 看板「当前正在执行」：$current"
  fi
  if [ -n "$status_short" ]; then
    echo ""
    echo "<details><summary>git status -u --short</summary>"
    echo ""
    echo '```'
    echo "$status_short"
    echo '```'
    echo ""
    echo "</details>"
  fi
  echo ""
  echo "<details><summary>最近 5 个 commit</summary>"
  echo ""
  echo '```'
  echo "$recent_log"
  echo '```'
  echo ""
  echo "</details>"
  echo ""
  echo "---"
} >> "$LOG"
