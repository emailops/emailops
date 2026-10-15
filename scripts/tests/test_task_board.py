"""The live board server: request authorization and launching tasks in Herdr."""

import pathlib
import subprocess
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
import task_board  # noqa: E402
from task_board import (  # noqa: E402
    agent_name,
    artifact_target,
    authorized,
    create_task,
    panes_in,
    post_params,
    pick_workspace,
    raise_command,
    read_args,
    refusal,
    Snapshot,
    start_args,
    sync_board,
    task_checkout,
    task_worktree,
    terminal_of,
)
from tasks import Task, render_html  # noqa: E402


def task(id="T-0001", status="todo"):
    return Task(id=id, title="A task", status=status, milestone="0.7.0", pr=None, body="Body.\n", file=f"{id}-a-task.md", worktree="main")


def render(tasks, **kw):
    return render_html(tasks, {}, repo="owner/repo", generated="13/10/2026 10:00", **kw)


# --- Herdr launcher -------------------------------------------------------


def test_agent_name_is_a_valid_herdr_name():
    assert agent_name("T-0007") == "t-0007"


def test_pick_workspace_prefers_the_focused_one_with_the_repo_label():
    ws = [
        {"workspace_id": "w1", "label": "emailopsv2", "focused": False},
        {"workspace_id": "w2", "label": "other", "focused": True},
        {"workspace_id": "w3", "label": "emailopsv2", "focused": True},
    ]
    assert pick_workspace(ws, "emailopsv2") == "w3"


def test_pick_workspace_falls_back_to_the_first_match():
    ws = [{"workspace_id": "w2", "label": "other", "focused": True}, {"workspace_id": "w4", "label": "emailopsv2", "focused": False}]
    assert pick_workspace(ws, "emailopsv2") == "w4"


def test_pick_workspace_without_a_match_is_none():
    assert pick_workspace([{"workspace_id": "w2", "label": "other", "focused": True}], "emailopsv2") is None


def test_task_worktree_is_named_after_the_task_file():
    path, branch = task_worktree("T-0002-chat-app-control-tools.md", pathlib.Path("/repo"))
    assert path == pathlib.Path("/repo/.claude/worktrees/T-0002-chat-app-control-tools")
    assert branch == "feature/T-0002-chat-app-control-tools"


def test_served_page_launches_through_the_server():
    html = render([task()], token="tok123")
    assert 'data-launch="T-0001"' in html
    assert "Open in Herdr" in html
    assert '"tok123"' in html


def test_served_page_refreshes_in_place_instead_of_reloading():
    html = render([task()], token="tok123")
    assert "location.reload()" not in html
    assert "DOMParser" in html


# --- authorized -----------------------------------------------------------


def req(method="GET", path="/?token=tok", host="127.0.0.1:8780", origin=None):
    headers = {"Host": host}
    if origin:
        headers["Origin"] = origin
    return authorized(method, path, headers, token="tok", port=8780)


def test_get_with_the_token_on_localhost_is_allowed():
    assert req()
    assert req(host="localhost:8780")


def test_missing_or_wrong_token_is_refused():
    assert not req(path="/")
    assert not req(path="/?token=nope")


def test_foreign_host_header_is_refused():
    # DNS rebinding: a page on evil.example resolving to 127.0.0.1 still sends its own Host.
    assert not req(host="evil.example:8780")


def test_post_needs_a_local_origin():
    assert req("POST", "/open?token=tok&task=T-0001", origin="http://127.0.0.1:8780")
    assert not req("POST", "/open?token=tok&task=T-0001")
    assert not req("POST", "/open?token=tok&task=T-0001", origin="https://evil.example")




# --- terminal_of ------------------------------------------------------------


def test_terminal_of_walks_up_from_the_herdr_client_to_its_app():
    procs = {
        2187: (2186, "/Users/u/.local/bin/herdr"),
        2186: (878, "herdr"),
        878: (877, "-/bin/zsh"),
        877: (741, "/usr/bin/login"),
        741: (1, "/Applications/cmux.app/Contents/MacOS/cmux"),
        1: (0, "/sbin/launchd"),
    }
    assert terminal_of(procs) == "/Applications/cmux.app"


def test_terminal_of_without_herdr_is_none():
    assert terminal_of({1: (0, "/sbin/launchd")}) is None


def test_cmux_is_raised_through_its_own_focus_command():
    # open -a activates cmux but stays on the current Space, so a full-screen window never shows.
    argv = raise_command("/Applications/cmux.app")
    assert argv[0] == "osascript"
    assert 'tell application "/Applications/cmux.app" to focus' in argv[-1]


def test_other_terminals_are_raised_with_open():
    assert raise_command("/Applications/Ghostty.app") == ["open", "-a", "/Applications/Ghostty.app"]


# --- read_args --------------------------------------------------------------


def test_an_idle_agent_is_read_with_its_scrollback():
    assert read_args("idle") == ("--source", "recent-unwrapped", "--lines", "80")
    assert read_args("done") == ("--source", "recent-unwrapped", "--lines", "80")


def test_a_busy_or_blocked_agent_is_read_from_its_screen():
    # Herdr refuses scrollback while the agent is not idle; the screen shows the pending question.
    for status in ("working", "blocked", "unknown"):
        assert read_args(status) == ("--source", "visible")


def test_the_token_may_come_from_the_board_cookie():
    # A report's relative assets (css, images) are fetched without the URL token.
    assert authorized("GET", "/a/T-0001/0/style.css", {"Host": "127.0.0.1:8780", "Cookie": "x=1; board=tok"}, token="tok", port=8780)
    assert not authorized("GET", "/a/T-0001/0/style.css", {"Host": "127.0.0.1:8780", "Cookie": "board=nope"}, token="tok", port=8780)


# --- artifact_target --------------------------------------------------------


def test_artifact_target_serves_the_file_and_its_siblings():
    base = "/w/reports/run1/index.html"
    assert artifact_target(base, "index.html") == pathlib.Path("/w/reports/run1/index.html")
    assert artifact_target(base, "assets/style.css") == pathlib.Path("/w/reports/run1/assets/style.css")


def test_artifact_target_refuses_to_leave_the_artifact_folder():
    for rel in ("../secret.txt", "assets/../../x", "/etc/passwd", ""):
        try:
            artifact_target("/w/reports/run1/index.html", rel)
        except ValueError:
            continue
        raise AssertionError(f"served {rel!r}")


# --- post_params ------------------------------------------------------------


def test_post_params_merge_the_query_and_the_form_body():
    assert post_params("task=T-0001", "title=Do+a+thing&body=") == {"task": "T-0001", "title": "Do a thing", "body": ""}


# --- discard ------------------------------------------------------------------


def test_panes_in_finds_every_pane_inside_the_task_worktree():
    panes = [
        {"pane_id": "p1", "cwd": "/r/.claude/worktrees/T-0002-x", "foreground_cwd": "/r/.claude/worktrees/T-0002-x/src"},
        {"pane_id": "p2", "cwd": "/r/.claude/worktrees/T-0002-x", "foreground_cwd": None},
        {"pane_id": "p3", "cwd": "/r", "foreground_cwd": "/r"},
        {"pane_id": "p4", "cwd": "/r/.claude/worktrees/T-0020-y", "foreground_cwd": None},
    ]
    assert panes_in(panes, "T-0002") == ["p1", "p2"]


def test_task_checkout_picks_only_the_task_worktree():
    trees = [
        (pathlib.Path("/r"), "main"),
        (pathlib.Path("/r/.claude/worktrees/tasks-board"), "chore/tasks-board"),
        (pathlib.Path("/r/.claude/worktrees/T-0002-x"), "feature/T-0002-x"),
    ]
    assert task_checkout("T-0002", trees) == (pathlib.Path("/r/.claude/worktrees/T-0002-x"), "feature/T-0002-x")
    assert task_checkout("T-0003", trees) is None


def test_task_checkout_ignores_a_task_branch_checked_out_outside_the_worktrees_dir():
    # The main checkout on a task branch must never be removed.
    assert task_checkout("T-0002", [(pathlib.Path("/r"), "feature/T-0002-x")]) is None


# --- board branch lifecycle (real git in a temp repo) ---------------------------


def sh(cwd, *args):
    return subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True, check=True).stdout.strip()


def repo_with_board(tmp_path):
    """A repo on main with tasks/ and a board worktree on chore/tasks-board."""
    root = tmp_path / "repo"
    (root / "tasks").mkdir(parents=True)
    sh(tmp_path, "init", "-q", "-b", "main", str(root))
    sh(root, "config", "user.email", "dev@example.com")
    sh(root, "config", "user.name", "Dev")
    (root / "tasks" / "milestones.md").write_text("- 0.7.0\n- later\n")
    (root / "tasks" / "T-0001-a.md").write_text("---\nid: T-0001\ntitle: A\nstatus: todo\nmilestone: later\npr: null\n---\n\nBody.\n")
    sh(root, "add", "-A")
    sh(root, "commit", "-q", "-m", "init")
    board = root / ".claude" / "worktrees" / "tasks-board"
    sh(root, "worktree", "add", "-q", "-b", "chore/tasks-board", str(board), "main")
    return root, board


def commit_file(cwd, name, text, message):
    (cwd / name).write_text(text)
    sh(cwd, "add", name)
    sh(cwd, "commit", "-q", "-m", message)


def test_a_merged_board_moves_up_to_the_base(tmp_path):
    root, board = repo_with_board(tmp_path)
    commit_file(board, "tasks/milestones.md", "- 0.7.0\n- 0.8.0\n- later\n", "board edit")
    sh(root, "merge", "-q", "--no-ff", "-m", "merge board", "chore/tasks-board")
    commit_file(root, "README", "x\n", "more work on main")
    sync_board(board, "main")
    assert sh(board, "rev-parse", "HEAD") == sh(root, "rev-parse", "main")


def test_a_board_with_unmerged_edits_stays_put(tmp_path):
    root, board = repo_with_board(tmp_path)
    commit_file(board, "tasks/milestones.md", "- 0.7.0\n- 0.8.0\n- later\n", "board edit")
    commit_file(root, "README", "x\n", "more work on main")
    head = sh(board, "rev-parse", "HEAD")
    sync_board(board, "main")
    assert sh(board, "rev-parse", "HEAD") == head


def test_create_task_commits_the_next_id_on_the_board(tmp_path, monkeypatch):
    root, board = repo_with_board(tmp_path)
    monkeypatch.chdir(root)
    monkeypatch.setattr(task_board, "TASK_BASE", "main")
    task_id, name = create_task("Do a thing", "0.7.0", "")
    assert (task_id, name) == ("T-0002", "tasks/T-0002-do-a-thing.md")
    assert sh(board, "log", "-1", "--format=%s") == "chore: add T-0002"
    assert "title: Do a thing" in (board / name).read_text()


# --- start modes ------------------------------------------------------------


def test_a_herdr_start_types_the_skill_without_extra_agent_args():
    assert start_args("T-0004", "herdr") == []


def test_a_background_start_runs_a_named_background_session():
    assert start_args("T-0004", "bg") == ["claude", "--bg", "-n", "T-0004", "/work-on-task T-0004"]


def test_a_cloud_start_passes_the_skill_to_a_cloud_session():
    assert start_args("T-0004", "cloud") == ["--cloud", "/work-on-task T-0004"]


def test_an_unknown_start_mode_is_refused():
    import pytest

    with pytest.raises(ValueError):
        start_args("T-0004", "teleport")


# --- Snapshot (the cached board the server hands out) -----------------------


class FakeBuild:
    def __init__(self, *versions):
        self.versions, self.calls = list(versions), 0

    def __call__(self):
        self.calls += 1
        v = self.versions[min(self.calls, len(self.versions)) - 1]
        return f"<page {v}>", v


def test_snapshot_builds_once_and_serves_the_cached_page():
    build = FakeBuild("v1")
    snap = Snapshot(build)
    assert snap.get() == ("<page v1>", "v1")
    assert snap.get() == ("<page v1>", "v1")
    assert build.calls == 1


def test_snapshot_refresh_reports_whether_the_board_changed():
    snap = Snapshot(FakeBuild("v1", "v1", "v2"))
    snap.get()
    assert snap.refresh() is False
    assert snap.refresh() is True
    assert snap.get()[1] == "v2"


def test_wait_returns_at_once_when_the_caller_is_behind():
    snap = Snapshot(FakeBuild("v1"))
    snap.get()
    assert snap.wait("old", timeout=5) == "v1"


def test_wait_wakes_up_on_a_change():
    import threading

    snap = Snapshot(FakeBuild("v1", "v2"))
    snap.get()
    threading.Timer(0.05, snap.refresh).start()
    assert snap.wait("v1", timeout=5) == "v2"


def test_wait_times_out_with_the_same_version():
    snap = Snapshot(FakeBuild("v1"))
    snap.get()
    assert snap.wait("v1", timeout=0.01) == "v1"


def test_snapshot_is_watched_only_while_a_page_asked_recently():
    now = [100.0]
    snap = Snapshot(FakeBuild("v1"), clock=lambda: now[0])
    assert snap.watched() is False
    snap.get()
    assert snap.watched() is True
    now[0] += 120
    assert snap.watched() is False


def test_a_failing_build_keeps_the_last_good_page():
    calls = []

    def build():
        calls.append(1)
        if len(calls) > 1:
            raise RuntimeError("git broke")
        return "<page v1>", "v1"

    snap = Snapshot(build)
    snap.get()
    import pytest

    with pytest.raises(RuntimeError):
        snap.refresh()
    assert snap.get() == ("<page v1>", "v1")


LEFTHOOK_FAILURE = """\
  File "scripts/tasks.py", line 454
SyntaxError: f-string: expecting '}'
INF no leaks found
summary: (done in 0.18 seconds)
\u2714\ufe0f no-invoke-outside-api (0.05 seconds)
\u2714\ufe0f gitleaks (0.08 seconds)
\U0001f94a docs-paths (0.01 seconds)
\U0001f94a tasks-check (0.03 seconds)
"""


def test_refusal_names_the_hooks_that_failed_not_the_last_ones_that_passed():
    assert refusal(LEFTHOOK_FAILURE, "/repo/.claude/worktrees/tasks-board") == (
        "commit refused by the pre-commit hook: docs-paths, tasks-check (git commit in /repo/.claude/worktrees/tasks-board shows why)"
    )


def test_refusal_without_a_hook_summary_keeps_the_end_of_the_output():
    assert refusal("fatal: unable to write index\n", "/repo") == "commit refused: fatal: unable to write index"
