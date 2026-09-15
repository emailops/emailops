#!/usr/bin/env bash
# Drive EmailOps for verification runs: launch an isolated dev instance on its
# own port and data dir, health-check it, snapshot it through cua-driver, and
# tear down exactly what this script started. Evidence goes to
# src-tauri/reports/verify/<run>/ (gitignored) and survives cleanup.
#
#   verify.sh launch            start the app (demo DB, port $VERIFY_PORT) and wait for its window
#   verify.sh doctor            is the instance this run started worth driving?
#   verify.sh snap <name>       AX tree + screenshot -> <run>/<name>.json/.png/.tree.txt
#   verify.sh find <name> <re>  element_index of elements whose role/label/value match <re>
#   verify.sh click <re> <name> snap, click the first element matching <re>, snap again
#   verify.sh type <re> <text> <name>  same, but type <text> into the matching element
#   verify.sh wd <cmd> …        DOM-level driving through the embedded WebDriver (see wd.mjs):
#                               status | find <sel> | text <sel> | exists <sel> | click <sel> |
#                               type <sel> <text> | keys <key> | js <expr> | shot <file.png>
#   verify.sh cleanup           kill the launcher tree + app pid recorded by launch; keep evidence
#
# Env: VERIFY_PORT (1421), VERIFY_DATA_DIR (<repo>/.emailops-demo-data),
#      VERIFY_RUN_DIR (<repo>/src-tauri/reports/verify/<timestamp>), VERIFY_LAUNCH_TIMEOUT (900s)
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
PORT="${VERIFY_PORT:-1421}"
WD_PORT="${TAURI_WEBDRIVER_PORT:-4445}"
WD="$(dirname "${BASH_SOURCE[0]}")/wd.mjs"
DATA_DIR="${VERIFY_DATA_DIR:-$REPO/.emailops-demo-data}"
RUNS_ROOT="$REPO/src-tauri/reports/verify"
CURRENT="$RUNS_ROOT/current"
PROD_DB="$HOME/Library/Application Support/com.emailops.app/emailops.db"

die() { echo "verify: $*" >&2; exit 1; }
need_run() {
  RUN_DIR="${VERIFY_RUN_DIR:-$(readlink "$CURRENT" 2>/dev/null || true)}"
  [ -n "$RUN_DIR" ] && [ -d "$RUN_DIR" ] || die "no active run (run 'verify.sh launch' first, or set VERIFY_RUN_DIR)"
  APP_PID="$(cat "$RUN_DIR/app.pid" 2>/dev/null || true)"
  WIN_ID="$(cat "$RUN_DIR/window.id" 2>/dev/null || true)"
  # `tauri dev` restarts the app whenever a Rust file changes; follow the new pid.
  if [ -n "$APP_PID" ] && ! kill -0 "$APP_PID" 2>/dev/null; then
    local fresh; fresh="$(find_app_pid || true)"
    if [ -n "$fresh" ]; then
      echo "verify: app restarted (pid $APP_PID -> $fresh); refreshing run state" >&2
      APP_PID="$fresh"; echo "$fresh" > "$RUN_DIR/app.pid"
      WIN_ID="$(main_window_id "$fresh")"; echo "$WIN_ID" > "$RUN_DIR/window.id"
    fi
  fi
}
main_window_id() {
  cua-driver list_windows "{\"pid\":$1}" 2>/dev/null | python3 -c '
import json,sys
try: ws=json.load(sys.stdin).get("windows",[])
except Exception: ws=[]
# the main window is often reported off-screen (other Space, behind the terminal): prefer on-screen, then titled, then largest
ws.sort(key=lambda w:(not w.get("is_on_screen"), not w.get("title"), -(w["bounds"]["width"]*w["bounds"]["height"])))
ws=[w for w in ws if w.get("title") or w.get("is_on_screen")]
print(ws[0]["window_id"] if ws else "")'
}
port_listener() { lsof -nP -iTCP:"$PORT" -sTCP:LISTEN -t 2>/dev/null | head -1; }
app_has_file() { lsof -p "$1" -Fn 2>/dev/null | grep -qxF "n$2"; }
find_app_pid() {
  for p in $(pgrep -x emailops 2>/dev/null); do
    if app_has_file "$p" "$DATA_DIR/emailops.db"; then echo "$p"; return 0; fi
  done
  return 1
}
descendants() { # all descendant pids of $1, depth-first
  for c in $(pgrep -P "$1" 2>/dev/null); do descendants "$c"; echo "$c"; done
}

cmd_launch() {
  case "$DATA_DIR" in *"Application Support/com.emailops.app"*) die "refusing to drive the production data dir";; esac
  # Reuse a healthy instance from a previous launch: relaunching relinks the app
  # crate (minutes) whenever HEAD or the feature set changed since the last build.
  if RUN_DIR="$(readlink "$CURRENT" 2>/dev/null)" && [ -n "$RUN_DIR" ] && [ -f "$RUN_DIR/app.pid" ] \
     && kill -0 "$(cat "$RUN_DIR/app.pid")" 2>/dev/null && [ "$(cat "$RUN_DIR/data_dir" 2>/dev/null)" = "$DATA_DIR" ] \
     && app_has_file "$(cat "$RUN_DIR/app.pid")" "$DATA_DIR/emailops.db" \
     && TAURI_WEBDRIVER_PORT="$(cat "$RUN_DIR/wd_port" 2>/dev/null || echo "$WD_PORT")" node "$WD" status >/dev/null 2>&1; then
    echo "ready (reused): pid=$(cat "$RUN_DIR/app.pid") webdriver=$(cat "$RUN_DIR/wd_port") run_dir=$RUN_DIR"
    return 0
  fi
  if [ -n "$(port_listener)" ]; then die "port $PORT is already in use by pid $(port_listener); not started by this run — pick another VERIFY_PORT"; fi
  if find_app_pid >/dev/null; then die "an emailops process already has $DATA_DIR/emailops.db open (pid $(find_app_pid)); refusing to double-drive"; fi
  if [ "$DATA_DIR" = "$REPO/.emailops-demo-data" ]; then
    (cd "$REPO" && bash scripts/ensure_demo_db.sh "$DATA_DIR" demo-db demo-embed)
  fi
  RUN_DIR="${VERIFY_RUN_DIR:-$RUNS_ROOT/$(date +%Y%m%d-%H%M%S)}"
  mkdir -p "$RUN_DIR"; ln -sfn "$RUN_DIR" "$CURRENT"
  echo "$DATA_DIR" > "$RUN_DIR/data_dir"; echo "$PORT" > "$RUN_DIR/port"
  local cfg="{\"build\":{\"devUrl\":\"http://localhost:$PORT\",\"beforeDevCommand\":\"npm run dev -- --port $PORT --strictPort\"}}"
  if lsof -nP -iTCP:"$WD_PORT" -sTCP:LISTEN -t >/dev/null 2>&1; then die "WebDriver port $WD_PORT is already in use; set TAURI_WEBDRIVER_PORT to a free port"; fi
  echo "$WD_PORT" > "$RUN_DIR/wd_port"
  # Start npm in its own session (setsid) so that whatever invoked launch — a
  # monitor, a tool timeout, a closed terminal — cannot take the app down with it.
  EMAILOPS_DATA_DIR="$DATA_DIR" TAURI_WEBDRIVER_PORT="$WD_PORT" python3 - "$REPO" "$RUN_DIR" "$cfg" <<'PY'
import os, subprocess, sys
repo, run_dir, cfg = sys.argv[1:4]
log = open(os.path.join(run_dir, "app.log"), "ab")
p = subprocess.Popen(["npm", "run", "tauri", "dev", "--", "--features", "webdriver", "--config", cfg],
                     cwd=repo, stdout=log, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, start_new_session=True)
open(os.path.join(run_dir, "launcher.pid"), "w").write(str(p.pid))
PY
  local launcher; launcher="$(cat "$RUN_DIR/launcher.pid")"
  echo "$(date +%T) launcher pid=$launcher log=$RUN_DIR/app.log (relinks the app when HEAD or the feature set changed: ~1 min; cold build: minutes)"
  local deadline=$(( $(date +%s) + ${VERIFY_LAUNCH_TIMEOUT:-900} )) pid=""
  while [ -z "$pid" ]; do
    kill -0 "$launcher" 2>/dev/null || { tail -20 "$RUN_DIR/app.log"; die "launcher exited before the app came up"; }
    [ "$(date +%s)" -lt "$deadline" ] || die "timed out waiting for the app (see $RUN_DIR/app.log)"
    pid="$(find_app_pid || true)"; [ -n "$pid" ] || { sleep 3; printf '.'; }
  done; echo
  echo "$pid" > "$RUN_DIR/app.pid"; echo "$(date +%T) app process up: pid=$pid"
  local wd_deadline=$(( $(date +%s) + 120 ))
  until TAURI_WEBDRIVER_PORT="$WD_PORT" node "$WD" status >>"$RUN_DIR/launch.trace" 2>&1; do
    [ "$(date +%s)" -lt "$wd_deadline" ] || { tail -5 "$RUN_DIR/launch.trace"; die "app is up but the WebDriver server never answered on $WD_PORT (see $RUN_DIR/launch.trace and app.log)"; }
    sleep 2
  done
  echo "$(date +%T) webdriver answering on $WD_PORT"
  # The window id only matters for the cua-driver layer; do not hold the run for it.
  local win="" tries=0
  while [ -z "$win" ] && [ "$tries" -lt 15 ]; do win="$(main_window_id "$pid")"; [ -n "$win" ] || { sleep 2; tries=$((tries+1)); }; done
  [ -n "$win" ] && echo "$win" > "$RUN_DIR/window.id" || echo "$(date +%T) no window listed by cua-driver yet (WebDriver driving still works; doctor will re-check)"
  echo "ready: pid=$pid window_id=$win port=$PORT webdriver=$WD_PORT data_dir=$DATA_DIR run_dir=$RUN_DIR"
}

cmd_doctor() {
  need_run; local ok=1
  check() { if eval "$2"; then echo "[ok  ] $1"; else echo "[FAIL] $1"; ok=0; fi; }
  local launcher; launcher="$(cat "$RUN_DIR/launcher.pid" 2>/dev/null || echo 0)"
  check "launcher pid $launcher alive"                    "kill -0 $launcher 2>/dev/null"
  check "app pid ${APP_PID:-?} is an emailops process"     "[ -n \"$APP_PID\" ] && [ \"\$(ps -o comm= -p $APP_PID 2>/dev/null | xargs basename 2>/dev/null)\" = emailops ]"
  check "app has $DATA_DIR/emailops.db open"               "app_has_file $APP_PID '$DATA_DIR/emailops.db'"
  check "app does NOT have the production DB open"         "! app_has_file $APP_PID '$PROD_DB'"
  check "port $PORT has a listener"                        "[ -n \"\$(port_listener)\" ]"
  check "WebDriver server answers on $WD_PORT"          "node '$WD' status >/dev/null 2>&1"
  check "cua-driver daemon running"                        "(cua-driver status 2>&1 || true) | grep -q running"
  check "cua-driver has Accessibility + Screen Recording"  "cua-driver check_permissions '{\"prompt\":false}' 2>/dev/null | python3 -c 'import json,sys; d=json.load(sys.stdin); sys.exit(0 if d.get(\"accessibility\") and d.get(\"screen_recording\") else 1)'"
  check "window ${WIN_ID:-?} still listed for pid"         "cua-driver list_windows '{\"pid\":$APP_PID}' 2>/dev/null | grep -q '\"window_id\": *$WIN_ID'"
  check "window is on the current Space (AX exposes it)"   "osascript -e 'tell application \"System Events\" to tell process \"emailops\" to get count of windows' 2>/dev/null | grep -qvx 0"
  [ "$ok" = 1 ] || echo "hint: an AX-invisible window usually means the app opened on another Space (your terminal is full-screen). Switch to that Space or leave full-screen, then re-run doctor; the driver cannot see web content on an off-Space window."
  [ "$ok" = 1 ] || exit 1
}

cmd_snap() {
  need_run; local name="${1:?snap needs a name}"
  cua-driver get_window_state "{\"pid\":$APP_PID,\"window_id\":$WIN_ID,\"max_elements\":3000}" \
    --screenshot-out-file "$RUN_DIR/$name.png" > "$RUN_DIR/$name.json"
  python3 - "$RUN_DIR/$name.json" "$RUN_DIR/$name.tree.txt" <<'PY'
import json,sys
d=json.load(open(sys.argv[1]))
open(sys.argv[2],'w').write(d.get('tree_markdown',''))
print(f"snap: elements={len(d.get('elements',[]))} screenshot={d.get('screenshot_width')}x{d.get('screenshot_height')}")
PY
  echo "      $RUN_DIR/$name.png"
}

cmd_find() {
  need_run; local name="${1:?find needs a snap name}" re="${2:?find needs a regex}"
  python3 - "$RUN_DIR/$name.json" "$re" <<'PY'
import json,re,sys
d=json.load(open(sys.argv[1])); pat=re.compile(sys.argv[2], re.I)
hits=[e for e in d.get('elements',[]) if pat.search(' '.join(str(e.get(k) or '') for k in ('role','label','value')))]
for e in hits: print(e['element_index'], e.get('role'), repr(e.get('label') or ''), repr(e.get('value') or '')[:60])
sys.exit(0 if hits else 1)
PY
}

first_index() { cmd_find "$1" "$2" | head -1 | cut -d' ' -f1; }

cmd_click() {
  need_run; local re="${1:?click needs a regex}" name="${2:?click needs an evidence name}"
  cmd_snap "$name-before" >/dev/null
  local idx; idx="$(first_index "$name-before" "$re")"; [ -n "$idx" ] || die "no element matches /$re/ in $name-before"
  echo "click: [$idx] $(cmd_find "$name-before" "$re" | head -1 | cut -d' ' -f2-)"
  cua-driver click "{\"pid\":$APP_PID,\"window_id\":$WIN_ID,\"element_index\":$idx}" >/dev/null
  sleep "${VERIFY_SETTLE:-1.5}"; cmd_snap "$name-after"
}

cmd_type() {
  need_run; local re="${1:?type needs a regex}" text="${2:?type needs text}" name="${3:?type needs an evidence name}"
  cmd_snap "$name-before" >/dev/null
  local idx; idx="$(first_index "$name-before" "$re")"; [ -n "$idx" ] || die "no element matches /$re/ in $name-before"
  cua-driver click "{\"pid\":$APP_PID,\"window_id\":$WIN_ID,\"element_index\":$idx}" >/dev/null; sleep 0.5
  local payload; payload="$(python3 -c 'import json,sys; print(json.dumps({"pid":int(sys.argv[1]),"window_id":int(sys.argv[2]),"element_index":int(sys.argv[3]),"text":sys.argv[4]}))' "$APP_PID" "$WIN_ID" "$idx" "$text")"
  cua-driver type_text "$payload" >/dev/null
  sleep "${VERIFY_SETTLE:-1.5}"; cmd_snap "$name-after"
}

cmd_cleanup() {
  need_run
  local launcher; launcher="$(cat "$RUN_DIR/launcher.pid" 2>/dev/null || true)"
  local victims=""
  [ -n "$launcher" ] && kill -0 "$launcher" 2>/dev/null && victims="$(descendants "$launcher") $launcher"
  [ -n "$APP_PID" ] && kill -0 "$APP_PID" 2>/dev/null && victims="$victims $APP_PID"
  for p in $victims; do
    # only ever kill what this run started: our launcher tree and the app that holds our DB
    case "$(ps -o command= -p "$p" 2>/dev/null)" in
      *"tauri dev"*|*"vite"*|*"npm run"*|*target/debug/emailops*|*"npm exec"*|*"node "*) kill "$p" 2>/dev/null || true;;
    esac
  done
  for _ in 1 2 3 4 5 6 7 8 9 10; do [ -z "$(port_listener)" ] && ! kill -0 "${APP_PID:-0}" 2>/dev/null && break; sleep 1; done
  for p in $victims; do kill -9 "$p" 2>/dev/null || true; done
  [ -z "$(port_listener)" ] || die "port $PORT still has a listener after cleanup"
  echo "cleanup: instance stopped; evidence kept in $RUN_DIR"
  ls -1 "$RUN_DIR" | sed 's/^/      /'
}

cmd_wd() { need_run; TAURI_WEBDRIVER_PORT="$(cat "$RUN_DIR/wd_port" 2>/dev/null || echo "$WD_PORT")" node "$WD" "$@"; }

case "${1:-}" in
  wd) shift; cmd_wd "$@";;
  launch) shift; cmd_launch "$@";;
  doctor) shift; cmd_doctor "$@";;
  snap) shift; cmd_snap "$@";;
  find) shift; cmd_find "$@";;
  click) shift; cmd_click "$@";;
  type) shift; cmd_type "$@";;
  cleanup) shift; cmd_cleanup "$@";;
  *) sed -n '2,15p' "$0"; exit 2;;
esac
