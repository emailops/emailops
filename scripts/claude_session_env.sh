#!/usr/bin/env bash
# SessionStart hook (.claude/settings.json): put the .nvmrc Node first on PATH
# for every Bash command of a Claude Code session. Claude Code sources
# $CLAUDE_ENV_FILE before each command, so agents no longer prepend the nvm
# path by hand. Does nothing without CLAUDE_ENV_FILE or a matching install;
# the frontend gates then report the missing Node themselves.
[[ -n "${CLAUDE_ENV_FILE:-}" ]] || exit 0
bin="$(bash "$(dirname "$0")/gates.sh" node-bin)" || exit 0
printf 'export PATH="%s:$PATH"\n' "$bin" >> "$CLAUDE_ENV_FILE"
