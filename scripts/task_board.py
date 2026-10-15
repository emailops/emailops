#!/usr/bin/env python3
"""Serve the task board live on 127.0.0.1 and act on it through Herdr.

Usage:
  task_board.py [--port N]
      Print the board's URL (a random token in it guards every request) and
      serve it until Ctrl+C. While a page watches it, the server rebuilds the
      board in the background (the tasks, see tasks.py, and the coding agents
      Herdr runs here and on its saved machines) every REBUILD_SECONDS and
      announces each change on /events, so the page swaps in a new status
      within a second or two; requests read the cached board. From the page:
      - Open in Herdr: focus the task's agent, or create (or reuse) the task's
        worktree, open a Herdr tab there, start Claude and type
        /work-on-task T-NNNN without sending it. Starting in the worktree, not
        the main checkout, means the session sees the repo's skills whatever
        branch the main checkout is on.
      - needs you: focus the agent waiting for input and raise the terminal
        app Herdr runs in.
      - Details: the agent's latest output and the artifacts the session
        recorded (tasks.py artifact), served from /a/T-NNNN/<n>/... so an HTML
        report keeps its relative assets.
      - New task, new milestone, a task's milestone, Delete (hides the task
        everywhere) and Discard worktree (removes the task's panes, worktree
        and local branch). Edits are committed on chore/tasks-board (its
        worktree is .claude/worktrees/tasks-board, created on first use and
        fast-forwarded to TASK_BASE once its edits are merged), or on the
        task's own branch once it is started. Nothing is pushed.
  task_board.py new --title TEXT --milestone NAME [--body-file PATH|-]
      Commit a new todo task on chore/tasks-board and print its id; the
      new-task skill uses it for "create a task for ...".
  task_board.py list [--status S]... [--all] [--milestone M] [--needs-you] [--json]
  task_board.py show T-NNNN [--json]
  task_board.py summary [--json]
      The board from a terminal or a Claude session (the tasks skill): open
      tasks unless --status/--all, one task with its Markdown and artifacts,
      or one line of counts and the tasks whose agent waits for you.
  task_board.py open T-NNNN [--mode herdr|bg|cloud]
      Start or focus a task's session: a Herdr tab in its worktree (the
      board's Open in Herdr), a background Claude session there (claude
      attach T-NNNN), or a cloud session in a Herdr tab.
  task_board.py focus|delete T-NNNN, milestone T-NNNN NAME, new-milestone NAME,
  discard T-NNNN [--yes]
      The board's other buttons. discard without --yes only says what it
      would remove.
"""

from __future__ import annotations

import argparse
import hmac
import json
import mimetypes
import os
import re
import secrets
import shutil
import socket
import subprocess
import sys
import threading
import time
from http.cookies import SimpleCookie
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, unquote, urlsplit

from tasks import (
    BOARD,
    BOARD_BRANCH,
    ID_RE,
    OPEN,
    STATUSES,
    TASK_BASE,
    TASK_BRANCH_RE,
    add_milestone,
    agent_label,
    agents_by_task,
    board_summary,
    build_page,
    collect,
    edit_location,
    filter_rows,
    format_rows,
    git,
    loss_text,
    main_root,
    merged_open,
    new_task_text,
    next_id,
    parse_task,
    read_milestones,
    set_field,
    slugify,
    task_of_path,
    task_rows,
    worktrees,
)

PORT = 8780
FETCH_SECONDS = 60  # how often the server fetches origin, to see task branches pushed elsewhere
REBUILD_SECONDS = 1.5  # how often a watched board is rebuilt in the background
WATCH_SECONDS = 60  # a board nobody asked for in this long is not rebuilt
EVENT_SECONDS = 15  # an /events keep-alive at least this often
OUTPUT_LINES = 80
MAX_FORM = 64 * 1024  # bytes of a POST body (a new task's details)


class Snapshot:
    """The last board page built, rebuilt in the background while a page watches it.

    Requests read the cached page instead of re-running git and Herdr, and
    /events wakes the pages as soon as a rebuild changes the board.
    build() returns (page, version).
    """

    def __init__(self, build, clock=time.monotonic):
        self.build, self.clock = build, clock
        self.page, self.version, self.asked = None, None, None
        self.changed = threading.Condition()

    def get(self):
        self.asked = self.clock()
        if self.page is None:
            self.refresh()
        return self.page, self.version

    def refresh(self):
        """Rebuild now; whether the board changed. A failed build keeps the last good page."""
        page, version = self.build()
        with self.changed:
            moved = version != self.version
            self.page, self.version = page, version
            if moved:
                self.changed.notify_all()
        return moved

    def wait(self, seen, timeout):
        """The current version once it differs from seen, or seen after timeout."""
        self.asked = self.clock()
        with self.changed:
            self.changed.wait_for(lambda: self.version != seen, timeout)
            return self.version

    def watched(self):
        return self.asked is not None and self.clock() - self.asked < WATCH_SECONDS


def task_worktree(file_name, root):
    """(path, branch) of a task's worktree, named after its file: T-0002-slug.md. Pure."""
    stem = Path(file_name).stem
    return root / ".claude" / "worktrees" / stem, f"feature/{stem}"


def agent_name(task_id):
    """Herdr agent name for a task (names are lowercase)."""
    return task_id.lower()


START_MODES = ("herdr", "bg", "cloud")


def start_args(task_id, mode):
    """How a task's session starts, by mode. Pure.

    herdr: no extra arguments (Claude starts in a Herdr tab and the skill is typed, not sent).
    bg: the full command of a named background session in the task's worktree.
    cloud: Claude's arguments for a cloud session, started in a Herdr tab so it stays attached.
    """
    prompt = f"/work-on-task {task_id}"
    if mode == "herdr":
        return []
    if mode == "bg":
        return ["claude", "--bg", "-n", task_id, prompt]
    if mode == "cloud":
        return ["--cloud", prompt]
    raise ValueError(f"start mode {mode!r}: use one of {', '.join(START_MODES)}")


def pick_workspace(workspaces, label):
    """Id of the Herdr workspace labelled after the repo, the focused one first. Pure."""
    hits = [w for w in workspaces if w["label"] == label]
    hits.sort(key=lambda w: not w["focused"])
    return hits[0]["workspace_id"] if hits else None


def authorized(method, path, headers, token, port):
    """Whether a request may reach the served board. Pure.

    Any page open in the browser can send requests to 127.0.0.1, and the board can
    start agents: require the URL token, a local Host (a rebound DNS name keeps its
    own), and on POST a local Origin.
    """
    local = {f"127.0.0.1:{port}", f"localhost:{port}"}
    cookie = SimpleCookie(headers.get("Cookie", "")).get("board")
    # The cookie covers what the page links without the token: an artifact's css, images.
    given = parse_qs(urlsplit(path).query).get("token", [cookie.value if cookie else ""])[0]
    if headers.get("Host") not in local or not hmac.compare_digest(given, token):
        return False
    return method != "POST" or headers.get("Origin", "").removeprefix("http://") in local


def post_params(query, body):
    """Form fields of a POST: the query string plus a urlencoded body. Pure."""
    params = {k: v[0] for k, v in parse_qs(query).items()}
    params.update({k: v[0] for k, v in parse_qs(body, keep_blank_values=True).items()})
    return params


def panes_in(panes, task_id):
    """Herdr pane ids whose shell sits inside the task's worktree. Pure."""
    return [p["pane_id"] for p in panes if task_id in (task_of_path(p.get("foreground_cwd") or ""), task_of_path(p.get("cwd") or ""))]


def task_checkout(task_id, trees):
    """(path, branch) of the task's worktree under .claude/worktrees, else None. Pure.

    Both the branch and the directory must name the task: a task branch checked
    out anywhere else (the main checkout) is never a candidate for removal.
    """
    for path, branch in trees:
        m = TASK_BRANCH_RE.match(branch)
        if m and m.group(1) == task_id and task_of_path(str(path)) == task_id:
            return path, branch
    return None


def artifact_target(artifact, rel):
    """The file /a/<task>/<n>/<rel> serves: the artifact or a file beside it. Pure."""
    base = Path(artifact).parent
    target = Path(os.path.normpath(base / rel)) if rel else base
    if not rel or Path(rel).is_absolute() or base not in target.parents:
        raise ValueError(f"not inside the artifact's folder: {rel!r}")
    return target


def read_args(status):
    """herdr agent read arguments for an agent in this state. Pure.

    Herdr captures scrollback only while the agent is idle; otherwise read the
    screen, which for a blocked agent shows the question it waits on.
    """
    if status in ("idle", "done"):
        return ("--source", "recent-unwrapped", "--lines", str(OUTPUT_LINES))
    return ("--source", "visible")


def terminal_of(procs):
    """The .app that hosts the Herdr client, from {pid: (ppid, command)}. Pure."""
    for pid, (_, command) in procs.items():
        if Path(command).name != "herdr":
            continue
        seen = set()
        while pid in procs and pid not in seen:
            seen.add(pid)
            pid, command = procs[pid]
            if ".app/Contents/MacOS/" in command:
                return command.split(".app/Contents/MacOS/")[0] + ".app"
    return None


def raise_command(app):
    """The command that brings the terminal app to the front. Pure.

    `open -a` activates cmux on the current Space, so a full-screen cmux window
    stays hidden behind the desktop; cmux's own `focus` switches to its Space.
    """
    if Path(app).stem == "cmux":
        script = f'tell application "{app}" to focus (focused terminal of selected tab of front window)'
        return ["osascript", "-e", script]
    return ["open", "-a", app]


# --- I/O ------------------------------------------------------------------


def herdr(*args, raw=False):
    """Run a Herdr CLI command and return its JSON reply (or its text with raw)."""
    exe = shutil.which("herdr") or str(Path.home() / ".local" / "bin" / "herdr")
    proc = subprocess.run([exe, *map(str, args)], capture_output=True, text=True)
    if proc.returncode != 0:
        raise RuntimeError(f"herdr {' '.join(map(str, args))}: {proc.stderr.strip() or proc.returncode}")
    if raw:
        return proc.stdout
    return json.loads(proc.stdout) if proc.stdout.strip() else {}


def on_machine(machine_id, *args):
    return ("--machine", machine_id, *args) if machine_id else args


def local_machine():
    try:
        return subprocess.run(["scutil", "--get", "ComputerName"], capture_output=True, text=True, check=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return socket.gethostname()


def live_agents():
    """(task id -> AgentInfo, warnings) from Herdr here and on its enabled saved machines."""
    try:
        entries = [(local_machine(), None, a) for a in herdr("agent", "list")["result"]["agents"]]
        machines = herdr("machine", "list", "--json")
    except (OSError, RuntimeError) as err:
        return {}, [f"Herdr unavailable, live agents not shown: {err}"]
    warnings = []
    for m in machines:
        if not m.get("enabled"):
            continue
        try:
            entries += [(m["label"], m["id"], a) for a in herdr("--machine", m["id"], "agent", "list")["result"]["agents"]]
        except RuntimeError as err:
            warnings.append(f"Herdr machine {m['label']} unreachable: {err}")
    return agents_by_task(entries), warnings


def agent_of(task_id):
    agent = live_agents()[0].get(task_id)
    if agent is None:
        raise ValueError(f"no live agent for {task_id}")
    return agent


def raise_terminal():
    """Bring the app that hosts the Herdr client to the front (best effort: it may be closed)."""
    out = subprocess.run(["ps", "-axo", "pid=,ppid=,comm="], capture_output=True, text=True, check=True).stdout
    procs = {}
    for line in out.splitlines():
        pid, ppid, command = line.split(None, 2)
        procs[int(pid)] = (int(ppid), command)
    app = terminal_of(procs)
    if app:
        subprocess.run(raise_command(app), check=True)


def focus(task_id):
    agent = agent_of(task_id)
    herdr(*on_machine(agent.machine_id, "agent", "focus", agent.pane_id))
    raise_terminal()


def agent_output(task_id):
    agent = agent_of(task_id)
    return herdr(*on_machine(agent.machine_id, "agent", "read", agent.pane_id, *read_args(agent.status)), raw=True)


def branch_exists(branch):
    return subprocess.run(["git", "rev-parse", "--verify", "-q", f"refs/heads/{branch}"], capture_output=True).returncode == 0


def find_task(task_id):
    """(main checkout, task, milestones) as the board shows them now."""
    if not ID_RE.match(task_id):
        raise ValueError(f"not a task id: {task_id!r}")
    root, tasks, _, milestones, _, _ = collect()
    task = next((t for t in tasks if t.id == task_id), None)
    if task is None:
        raise ValueError(f"no task {task_id}")
    return root, task, milestones


def board_worktree(root):
    """The checkout the board commits to: created off TASK_BASE on first use, brought up to it once merged."""
    path = root / ".claude" / "worktrees" / BOARD
    if not path.exists():
        if branch_exists(BOARD_BRANCH):
            git("worktree", "add", str(path), BOARD_BRANCH)
        else:
            git("worktree", "add", "-b", BOARD_BRANCH, str(path), TASK_BASE)
    sync_board(path, TASK_BASE)
    return path


def sync_board(path, base):
    """Fast-forward the board branch to base when every edit on it is already merged.

    PRs land as merge commits here, so a merged board is an ancestor of base.
    A board with unmerged edits is left alone: those go to main through a PR.
    """
    if subprocess.run(["git", "merge-base", "--is-ancestor", "HEAD", base], cwd=path, capture_output=True).returncode == 0:
        git("merge", "-q", "--ff-only", base, cwd=path)


def commit(cwd, path, message):
    """Commit one already-staged path; on failure put it back as it was."""
    try:
        git("commit", "-q", "-m", message, "--", path, cwd=cwd)
    except subprocess.CalledProcessError as err:
        git("reset", "-q", "HEAD", "--", path, cwd=cwd)
        git("checkout", "-q", "HEAD", "--", path, cwd=cwd)
        raise ValueError(refusal((err.stdout or "") + (err.stderr or ""), cwd)) from err


def refusal(output, cwd):
    """Why a commit was refused: the hooks lefthook marks failed (🥊), else the end of the output."""
    failed = re.findall(r"^\U0001f94a (\S+)", output, re.MULTILINE)
    if failed:
        return f"commit refused by the pre-commit hook: {', '.join(failed)} (git commit in {cwd} shows why)"
    return f"commit refused: {output.strip()[-400:]}"


def new_milestone(query):
    name = query.get("name", "")
    board = board_worktree(main_root())
    file = board / "tasks" / "milestones.md"
    file.write_text(add_milestone(file.read_text(encoding="utf-8"), name), encoding="utf-8")
    git("add", "tasks/milestones.md", cwd=board)
    commit(board, "tasks/milestones.md", f"chore: add milestone {name}")


def set_milestone(query):
    root, task, milestones = find_task(query.get("task", ""))
    name = query.get("name", "")
    if name not in milestones:
        raise ValueError(f"no milestone {name!r}; create it first")
    where = edit_location(task)
    cwd = board_worktree(root) if where == BOARD else root / ".claude" / "worktrees" / where
    file = cwd / "tasks" / task.file
    text = file.read_text(encoding="utf-8")
    if parse_task(text, task.file, where).milestone == name:
        return
    file.write_text(set_field(text, "milestone", name), encoding="utf-8")
    git("add", f"tasks/{task.file}", cwd=cwd)
    commit(cwd, f"tasks/{task.file}", f"chore: move {task.id} to milestone {name}")


def new_task(query):
    create_task(query.get("title", ""), query.get("milestone", ""), query.get("body", ""))


def create_task(title, milestone, body):
    """Commit a new todo task on the board branch; (id, path in the repo)."""
    board = board_worktree(main_root())
    if milestone not in read_milestones(board):
        raise ValueError(f"no milestone {milestone!r}; create it first")
    # Every id any branch ever had, so a deleted task's id (and its old branches) is never reused.
    used = re.findall(r"tasks/(T-\d{4})-", git("log", "--all", "--format=", "--name-only", "--", "tasks/", cwd=board))
    task_id = next_id(used + [f.name[:6] for f in (board / "tasks").glob("T-*.md")])
    text = new_task_text(task_id, title, milestone, body)
    name = f"tasks/{task_id}-{slugify(title) or 'task'}.md"
    (board / name).write_text(text, encoding="utf-8")
    git("add", name, cwd=board)
    commit(board, name, f"chore: add {task_id}")
    return task_id, name


def delete_task(query):
    """Drop a task on the board branch; the board then hides it from every source.

    A started task keeps its worktree, branch and agent: deleting a card never
    discards code.
    """
    root, task, _ = find_task(query.get("task", ""))
    remove_card(root, task)


def remove_card(root, task):
    board = board_worktree(root)
    if not (board / "tasks" / task.file).exists():
        raise ValueError(f"{task.id} exists only in {task.worktree}; delete it there")
    git("rm", "-q", f"tasks/{task.file}", cwd=board)
    commit(board, f"tasks/{task.file}", f"chore: delete {task.id}")


def close_merged(prs):
    """Set done, on the board branch, every open task whose PR is in prs; the ids closed.

    Always the board, even for a started task: its own branch is merged and
    finished, so an edit there would never reach main.
    """
    root, tasks = collect()[:2]
    board = board_worktree(root)
    closed = []
    for task in merged_open(tasks, set(prs)):
        file = board / "tasks" / task.file
        if not file.exists():
            raise ValueError(f"{task.id} exists only in {task.worktree}; set it done there")
        file.write_text(set_field(file.read_text(encoding="utf-8"), "status", "done"), encoding="utf-8")
        git("add", f"tasks/{task.file}", cwd=board)
        commit(board, f"tasks/{task.file}", f"chore: {task.id} done (#{task.pr})")
        closed.append(task.id)
    return closed


def discard_task(query):
    """Remove everything of a task on this machine: card, Herdr panes, worktree (uncommitted work too), local branch.

    A pushed branch and its PR stay: removing those changes GitHub.
    """
    task_id = query.get("task", "")
    if not ID_RE.match(task_id):
        raise ValueError(f"not a task id: {task_id!r}")
    found = task_checkout(task_id, worktrees())
    if found is None:
        raise ValueError(f"{task_id} has no worktree on this machine")
    path, branch = found
    root, tasks = collect()[:2]
    task = next((t for t in tasks if t.id == task_id), None)
    if task is not None:
        remove_card(root, task)
    try:
        panes = herdr("pane", "list")["result"]["panes"]
    except (OSError, RuntimeError) as err:
        print(f"discard {task_id}: Herdr unavailable, no panes closed: {err}", file=sys.stderr)
        panes = []
    for pane in panes_in(panes, task_id):
        herdr("pane", "close", pane)
    git("worktree", "remove", "--force", str(path))
    git("branch", "-D", branch)


def ensure_worktree(task, root):
    """The task's worktree, created (or re-attached to its surviving branch) when missing.

    A new branch starts from the base with the board branch merged on top, so it
    carries its own task file and the milestones, and merging its PR brings the
    board's edits to main.
    """
    path, branch = task_worktree(task.file, root)
    cwd = next((p for p, b in worktrees() if b == branch), None)
    if cwd is None:
        if branch_exists(branch):
            git("worktree", "add", str(path), branch)  # the branch outlived its worktree
        else:
            board_worktree(root)  # brought up to the base first when its edits are merged
            git("worktree", "add", "-b", branch, str(path), TASK_BASE)
            merge_board(path)
        cwd = path
    return cwd


def merge_board(path):
    """Merge the board branch into the checkout at path; a conflict is undone and reported."""
    try:
        git("merge", "-q", "--no-edit", "-m", f"chore: merge {BOARD_BRANCH}", BOARD_BRANCH, cwd=path)
    except subprocess.CalledProcessError as err:
        subprocess.run(["git", "merge", "--abort"], cwd=path, capture_output=True)
        raise ValueError(f"merging {BOARD_BRANCH} into {path} failed: {(err.stdout or err.stderr).strip()[-400:]}") from err


def herdr_tab(root, cwd, label):
    """A new Herdr tab (or workspace) at cwd in the repo's workspace; its root pane id."""
    ws = pick_workspace(herdr("workspace", "list")["result"]["workspaces"], root.name)
    if ws is None:
        made = herdr("workspace", "create", "--cwd", cwd, "--label", root.name, "--focus")["result"]
    else:
        made = herdr("tab", "create", "--workspace", ws, "--cwd", cwd, "--label", label, "--focus")["result"]
        herdr("workspace", "focus", ws)
    return made["root_pane"]["pane_id"]


def open_task(task_id, mode="herdr"):
    """Start (or focus) a task's session; a message saying what happened.

    herdr: focus its live agent, or open a Herdr tab in its worktree with Claude
    and /work-on-task typed. bg: a background Claude session in the worktree
    (`claude attach T-NNNN`). cloud: a cloud session in a Herdr tab at the main
    checkout; the board sees it once it pushes feature/T-NNNN-*.
    """
    extra = start_args(task_id, mode)
    if mode == "herdr" and task_id in live_agents()[0]:
        focus(task_id)
        return f"{task_id}: focused its agent"
    root, task, _ = find_task(task_id)
    if mode == "cloud":
        pane = herdr_tab(root, root, f"{task_id} cloud")
        herdr("agent", "start", f"{agent_name(task_id)}-cloud", "--kind", "claude", "--pane", pane, "--", *extra)
        raise_terminal()
        return f"{task_id}: cloud session starting in a Herdr tab"
    cwd = ensure_worktree(task, root)
    if mode == "bg":
        out = subprocess.run(extra, cwd=cwd, capture_output=True, text=True)
        if out.returncode != 0:
            raise RuntimeError(f"claude --bg: {out.stderr.strip() or out.returncode}")
        return f"{task_id}: background session {out.stdout.strip()} in {cwd} (claude attach {task_id})"
    pane = herdr_tab(root, cwd, task_id)
    herdr("agent", "start", agent_name(task_id), "--kind", "claude", "--pane", pane)
    herdr("pane", "send-text", pane, f"/work-on-task {task_id}")
    raise_terminal()
    return f"{task_id}: Herdr tab opened in {cwd}"


def open_in_herdr(task_id):
    open_task(task_id, "herdr")


def fetch_forever():
    """Keep origin's task branches current, so work pushed from elsewhere shows up."""
    stop = threading.Event()
    while not stop.wait(FETCH_SECONDS):
        proc = subprocess.run(["git", "fetch", "-q", "--prune", "origin"], capture_output=True, text=True)
        if proc.returncode != 0:
            print(f"git fetch: {proc.stderr.strip()}", file=sys.stderr)


def rebuild_forever(snapshot):
    """Rebuild the board every REBUILD_SECONDS while a page watches it."""
    stop = threading.Event()
    while not stop.wait(REBUILD_SECONDS):
        if not snapshot.watched():
            continue
        try:
            snapshot.refresh()
        except (subprocess.CalledProcessError, OSError, ValueError, RuntimeError) as err:
            print(f"board rebuild: {err}", file=sys.stderr)


def serve(port):
    token = secrets.token_urlsafe(16)

    def build():
        agents, warnings = live_agents()
        return build_page(token, agents, warnings)

    snapshot = Snapshot(build)
    actions = {
        "/open": lambda q: open_task(q.get("task", ""), q.get("mode", "herdr")),
        "/focus": lambda q: focus(q.get("task", "")),
        "/milestone/new": new_milestone,
        "/milestone/set": set_milestone,
        "/delete": delete_task,
        "/task/new": new_task,
        "/discard": discard_task,
    }

    class Handler(BaseHTTPRequestHandler):
        def reply(self, code, body, kind, cookie=False):
            data = body if isinstance(body, bytes) else body.encode()
            self.send_response(code)
            self.send_header("Content-Type", kind)
            if cookie:
                self.send_header("Set-Cookie", f"board={token}; HttpOnly; SameSite=Strict; Path=/")
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(data)

        def task(self):
            return parse_qs(urlsplit(self.path).query).get("task", [""])[0]

        def do_GET(self):
            route = urlsplit(self.path).path
            known = route in ("/", "/output", "/events") or route.startswith("/a/")
            if not authorized("GET", self.path, self.headers, token, port) or not known:
                return self.reply(404, "not found", "text/plain")
            try:
                if route == "/events":
                    return self.events()
                if route == "/output":
                    return self.reply(200, agent_output(self.task()), "text/plain; charset=utf-8")
                if route.startswith("/a/"):
                    return self.artifact(route)
                self.reply(200, snapshot.get()[0], "text/html; charset=utf-8", cookie=True)
            except (subprocess.CalledProcessError, OSError, ValueError, RuntimeError) as err:
                print(f"GET {route}: {err}", file=sys.stderr)
                self.reply(500, f"failed: {err}", "text/plain")

        def events(self):
            """Server-sent events: the board's version now and after every change, until the page goes."""
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            seen = None
            try:
                while True:
                    current = snapshot.wait(seen, EVENT_SECONDS) if seen is not None else snapshot.get()[1]
                    self.wfile.write(f"data: {current}\n\n".encode() if current != seen else b": ping\n\n")
                    self.wfile.flush()
                    seen = current
            except (BrokenPipeError, ConnectionResetError):
                pass  # the page closed or reloaded

        def artifact(self, route):
            _, _, task_id, n, rel = (unquote(part) for part in route.split("/", 4))
            entries = collect()[5].get(task_id, [])
            if not n.isdigit() or int(n) >= len(entries):
                return self.reply(404, "no such artifact", "text/plain")
            target = artifact_target(entries[int(n)]["path"], rel)
            kind = mimetypes.guess_type(target.name)[0] or "application/octet-stream"
            self.reply(200, target.read_bytes(), kind)

        def do_POST(self):
            route = urlsplit(self.path).path
            if not authorized("POST", self.path, self.headers, token, port) or route not in actions:
                return self.reply(404, "not found", "text/plain")
            length = int(self.headers.get("Content-Length") or 0)
            if length > MAX_FORM:
                return self.reply(413, "too large", "text/plain")
            try:
                actions[route](post_params(urlsplit(self.path).query, self.rfile.read(length).decode()))
            except (ValueError, RuntimeError, subprocess.CalledProcessError) as err:
                print(f"POST {route} {self.task()}: {err}", file=sys.stderr)
                return self.reply(200, json.dumps({"ok": False, "error": str(err)}), "application/json")
            try:
                # Before the reply: the page re-reads the board on "ok" and must get the edit.
                snapshot.refresh()
            except (ValueError, RuntimeError, OSError, subprocess.CalledProcessError) as err:
                print(f"POST {route}: board rebuild after the edit: {err}", file=sys.stderr)
            self.reply(200, json.dumps({"ok": True}), "application/json")

        def log_message(self, *args):
            pass  # the page and its event stream ask often; errors are printed where they happen

    server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    server.daemon_threads = True  # open /events streams must not keep Ctrl+C waiting
    threading.Thread(target=fetch_forever, daemon=True).start()
    threading.Thread(target=rebuild_forever, args=(snapshot,), daemon=True).start()
    print(f"http://127.0.0.1:{port}/?token={token}  (Ctrl+C to stop)", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


def board_rows():
    """(rows, artifacts by task id, worktree infos, warnings) of the board as it is now."""
    agents, warnings = live_agents()
    _, tasks, infos, _, found, artifacts = collect()
    return task_rows(tasks, infos, agents), artifacts, infos, [*warnings, *found]


def emit(data, as_json, text):
    print(json.dumps(data, indent=2, ensure_ascii=False) if as_json else text)


def cmd_list(args):
    rows, _, _, warnings = board_rows()
    statuses = STATUSES if args.all else (args.status or OPEN)
    rows = filter_rows(rows, statuses, args.milestone, args.needs_you)
    emit({"tasks": rows, "warnings": warnings}, args.json, "\n".join([format_rows(rows), *(f"warning: {w}" for w in warnings)]))


def cmd_show(args):
    rows, artifacts, _, _ = board_rows()
    row = next((r for r in rows if r["id"] == args.task), None)
    if row is None:
        raise ValueError(f"no task {args.task}")
    _, task, _ = find_task(args.task)
    data = {**row, "body": task.body, "artifacts": artifacts.get(args.task, [])}
    head = [f"{row['id']} · {row['title']}", f"status {row['status']} · milestone {row['milestone']} · agent {agent_label(row)}"]
    if row["branch"]:
        head.append(f"{row['where']} branch {row['branch']} · {row['last_commit']} · {row['dirty']} changed · {row['ahead']} ahead")
    head += [f"artifact: {a['label']} ({a['path']})" for a in data["artifacts"]]
    emit(data, args.json, "\n".join(head) + "\n\n" + task.body)


def cmd_summary(args):
    s = board_summary(board_rows()[0])
    emit(s, args.json, s["text"])


def cmd_discard(args):
    found = task_checkout(args.task, worktrees())
    if found is None:
        raise ValueError(f"{args.task} has no worktree on this machine")
    if not args.yes:
        info = board_rows()[2].get(found[0].name)
        lost = loss_text(info) if info else "the worktree"
        print(f"{args.task}: discarding removes {found[0]} and branch {found[1]} with {lost}. Re-run with --yes to do it.")
        return 2
    discard_task({"task": args.task})
    print(f"{args.task}: worktree, branch and Herdr panes removed (a pushed branch and its PR stay)")
    return 0


def cmd_new(args):
    body = ""
    if args.body_file:
        body = sys.stdin.read() if args.body_file == "-" else Path(args.body_file).read_text(encoding="utf-8")
    task_id, name = create_task(args.title, args.milestone, body)
    print(f"{task_id} {name} (committed on {BOARD_BRANCH}; not pushed)")


def parser():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--port", type=int, default=PORT)
    sub = p.add_subparsers(dest="cmd")
    new_p = sub.add_parser("new", help="commit a new todo task on the board branch and print its id")
    new_p.add_argument("--title", required=True)
    new_p.add_argument("--milestone", required=True)
    new_p.add_argument("--body-file", help="Markdown details; '-' reads stdin; omit for the Context/Acceptance template")
    list_p = sub.add_parser("list", help="the board's tasks as a table (open ones unless --status/--all)")
    list_p.add_argument("--status", action="append", choices=STATUSES)
    list_p.add_argument("--all", action="store_true")
    list_p.add_argument("--milestone")
    list_p.add_argument("--needs-you", action="store_true", help="only tasks whose agent waits for you")
    list_p.add_argument("--json", action="store_true")
    show_p = sub.add_parser("show", help="one task: status, branch, agent, artifacts and its Markdown")
    show_p.add_argument("task")
    show_p.add_argument("--json", action="store_true")
    sum_p = sub.add_parser("summary", help="one line: open tasks by status and who waits for you")
    sum_p.add_argument("--json", action="store_true")
    open_p = sub.add_parser("open", help="start or focus a task's session")
    open_p.add_argument("task")
    open_p.add_argument("--mode", choices=START_MODES, default="herdr")
    sub.add_parser("focus", help="focus the task's agent and raise its terminal").add_argument("task")
    ms_p = sub.add_parser("milestone", help="move a task to another milestone")
    ms_p.add_argument("task")
    ms_p.add_argument("name")
    sub.add_parser("new-milestone", help="add a milestone to tasks/milestones.md").add_argument("name")
    sub.add_parser("delete", help="delete a task's card (its worktree and branch stay)").add_argument("task")
    close_p = sub.add_parser("close-merged", help="set done, on the board branch, the open tasks whose PR is one of these merged PRs")
    close_p.add_argument("prs", nargs="+", type=int)
    dis_p = sub.add_parser("discard", help="remove a task's worktree, local branch and Herdr panes")
    dis_p.add_argument("task")
    dis_p.add_argument("--yes", action="store_true", help="without it, only say what would be lost")
    return p


def main(argv=None):
    args = parser().parse_args(argv)
    if args.cmd is None:
        return serve(args.port)
    commands = {
        "new": cmd_new,
        "list": cmd_list,
        "show": cmd_show,
        "summary": cmd_summary,
        "discard": cmd_discard,
        "open": lambda a: print(open_task(a.task, a.mode)),
        "focus": lambda a: (focus(a.task), print(f"{a.task}: focused"))[-1],
        "milestone": lambda a: (set_milestone({"task": a.task, "name": a.name}), print(f"{a.task}: milestone {a.name}"))[-1],
        "new-milestone": lambda a: (new_milestone({"name": a.name}), print(f"milestone {a.name} added"))[-1],
        "delete": lambda a: (delete_task({"task": a.task}), print(f"{a.task}: card deleted"))[-1],
        "close-merged": lambda a: print("\n".join(f"{t}: done" for t in close_merged(a.prs)) or "no open task has one of these PRs"),
    }
    try:
        return commands[args.cmd](args) or 0
    except (ValueError, RuntimeError, OSError, subprocess.CalledProcessError) as err:
        print(f"error: {err}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
