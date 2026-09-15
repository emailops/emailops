#!/usr/bin/env bash
# Nightly verification on this Mac (the evals need the local model and GPU, so it
# cannot run in CI). `run` does one `make verify`, commits a Markdown summary under
# docs/verification/ (the HTML report stays local) and keeps the last KEEP local runs.
# Usage: scripts/verify_schedule.sh run|install|uninstall
set -euo pipefail
cd "$(dirname "$0")/.."
REPO="$(pwd)"
LABEL=com.emailops.verify-nightly
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
REPORTS=src-tauri/reports/verify
KEEP=10

prune() {
  local runs=() r i
  while IFS= read -r r; do runs+=("$r"); done < <(ls -1d "$1"/[0-9]*-full 2>/dev/null | sort)
  for ((i = 0; i < ${#runs[@]} - KEEP; i++)); do rm -rf -- "${runs[$i]}"; done
}

run() {
  echo "[$(date '+%d/%m/%Y %H:%M')] nightly verification"
  # Another EmailOps instance on the demo DB holds the GPU: every eval would fail
  # with Metal out-of-memory and the e2e launch refuses to double-drive it.
  if [ -n "$(lsof -t .emailops-demo-data/emailops.db 2>/dev/null)" ]; then
    echo "skip: an EmailOps process has the demo DB open"; return 0
  fi
  # Latest commit that changes more than the summaries, so committing a summary
  # does not trigger another run of the same code.
  local sha summary
  sha="$(git log -1 --format=%h --abbrev=7 -- . ':(exclude)docs/verification')"
  if [ -n "$(git ls-files "docs/verification/*-$sha.md")" ]; then
    echo "skip: $sha already has a summary"; return 0
  fi
  VERIFY_EVAL_MODEL=qwen3.6-35b-a3b-ud-q4_k_xl make verify
  summary="$(python3 .claude/skills/verify-emailops/scripts/summary_md.py "$REPORTS/current-full/results.json" docs/verification)"
  if [ "$(git branch --show-current)" = main ]; then
    echo "not committing on main: $summary"
  else
    git add -- "$summary"
    git commit -q -m "docs: verification summary $sha" -- "$summary"
    echo "committed $summary"
  fi
  prune "$REPORTS"
}

install() {
  mkdir -p "$HOME/Library/LaunchAgents" "$REPORTS"
  cat > "$PLIST" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key>
  <array><string>/bin/bash</string><string>$REPO/scripts/verify_schedule.sh</string><string>run</string></array>
  <key>WorkingDirectory</key><string>$REPO</string>
  <key>EnvironmentVariables</key><dict><key>PATH</key><string>$PATH</string></dict>
  <key>StartCalendarInterval</key><dict><key>Hour</key><integer>3</integer><key>Minute</key><integer>0</integer></dict>
  <key>StandardOutPath</key><string>$REPO/$REPORTS/nightly.log</string>
  <key>StandardErrorPath</key><string>$REPO/$REPORTS/nightly.log</string>
</dict>
</plist>
PLIST
  # bootout fails when the agent is not loaded yet; that is the expected first-install case.
  launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
  launchctl bootstrap "gui/$(id -u)" "$PLIST"
  echo "installed $LABEL: daily at 03:00, log $REPORTS/nightly.log"
}

uninstall() {
  launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
  rm -f "$PLIST"
  echo "uninstalled $LABEL"
}

case "${1:-}" in
  run) run ;;
  install) install ;;
  uninstall) uninstall ;;
  *) echo "usage: $0 run|install|uninstall" >&2; exit 2 ;;
esac
