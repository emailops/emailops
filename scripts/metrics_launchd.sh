#!/usr/bin/env bash
# Install a macOS LaunchAgent that dispatches the `metrics` workflow every day
# at 08:00 local time.
#
# The workflow's own `schedule:` trigger cannot do this: GitHub ran the 07:47
# UTC cron between 4 and 9 hours late on every one of its first 20 days, so the
# report landed in the afternoon. A local `launchd` job fires on time and
# `gh workflow run` starts the run within seconds. launchd uses the Mac's own
# clock, so 08:00 stays 08:00 across the DST change. If the Mac is asleep at
# 08:00 the job runs on wake; if it is off, that day falls back to the
# workflow's cron, which skips itself when the day already has a report.
#
# Re-running replaces the agent. Remove it with:
#   launchctl bootout gui/$(id -u)/com.emailops.metrics
#   rm ~/Library/LaunchAgents/com.emailops.metrics.plist

set -euo pipefail

LABEL=com.emailops.metrics
REPO=emailops/emailops
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
LOG="$HOME/Library/Logs/emailops-metrics.log"

gh_bin=$(command -v gh) || { echo "gh not found on PATH" >&2; exit 1; }
gh auth status >/dev/null || { echo "gh is not authenticated — run: gh auth login" >&2; exit 1; }

mkdir -p "$(dirname "$PLIST")" "$(dirname "$LOG")"
cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key>
  <array>
    <string>$gh_bin</string>
    <string>workflow</string>
    <string>run</string>
    <string>metrics.yml</string>
    <string>--repo</string>
    <string>$REPO</string>
  </array>
  <key>StartCalendarInterval</key>
  <dict>
    <key>Hour</key><integer>8</integer>
    <key>Minute</key><integer>0</integer>
  </dict>
  <key>StandardOutPath</key><string>$LOG</string>
  <key>StandardErrorPath</key><string>$LOG</string>
</dict>
</plist>
EOF

launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
launchctl bootstrap "gui/$(id -u)" "$PLIST"
echo "[metrics] installed $PLIST — dispatches metrics.yml daily at 08:00, log: $LOG"
