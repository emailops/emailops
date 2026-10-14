#!/usr/bin/env python3
"""Validate the task files under tasks/ and build the live task board page.

Every task is one Markdown file, tasks/T-NNNN-slug.md, with a frontmatter block
(id, title, status, milestone, pr) and a free-form body. The files are the
source of truth; there is no generated index in git, so agents working on
different tasks in different worktrees never touch the same file.

Usage:
  tasks.py check
      Validate the task files of this checkout (milestones must be listed in
      tasks/milestones.md). Exit 1 on any error.
  tasks.py artifact PATH [--label TEXT]
      From inside a task worktree: record a file the session produced (an eval
      report, a verification summary, a screenshot) in that worktree's
      .claude/task-artifacts.local.json (gitignored: absolute local paths).
      The board links the latest one on the card and lists them in Details.
  tasks.py prune-done
      git rm the done tasks of this checkout; the release skill runs it so a
      release commit drops the tasks it shipped.

The board page (served by scripts/task_board.py, make tasks-serve) layers, per
task, the most advanced status from: the base branch (TASK_BASE), the board's
own branch (chore/tasks-board: new tasks, milestone edits and deletions made
from the board), the task's own worktree (feature/T-NNNN-*, uncommitted edits
included) and a pushed feature/T-NNNN-* branch with no local worktree (another
machine or a cloud session). Other worktrees are ignored: each carries a stale
copy of every task. Task worktrees add their activity (branch, last commit,
dirty files, commits ahead of origin/main, minutes since the last change, and
whether an agent is on it).
"""

# The URL handler runs macOS /usr/bin/python3 (3.9): keep annotations lazy.
from __future__ import annotations

import argparse
import hashlib
import html
import json
import re
import subprocess
import sys
import time
import unicodedata
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from urllib.parse import quote

STATUSES = ("todo", "doing", "review", "done", "dropped")
# Merge order across worktrees: a later status means more progress.
RANK = {"todo": 0, "doing": 1, "review": 2, "dropped": 3, "done": 4}
FIELDS = ("id", "title", "status", "milestone", "pr")
ID_RE = re.compile(r"^T-\d{4}$")
MAIN = "main"  # label of the base branch's tasks
BOARD = "tasks-board"  # label and worktree dir of the branch the live board writes to
BOARD_BRANCH = "chore/tasks-board"
TASK_BRANCH_RE = re.compile(r"^(?:feature|fix|refactor)/(T-\d{4})-")
TASK_PATH_RE = re.compile(r"/\.claude/worktrees/(T-\d{4})-[^/]*(?:/|$)")
MILESTONE_RE = re.compile(r"^[0-9A-Za-z][0-9A-Za-z._-]{0,31}$")
ARTIFACTS = Path(".claude") / "task-artifacts.local.json"  # per task worktree, gitignored
MAX_ARTIFACTS = 20
NEEDS_YOU = ("blocked", "done")  # Herdr states where the agent waits for the user
ACTIVE_MINUTES = 10  # a worktree changed this recently counts as having an agent on it
POLL_SECONDS = 30  # fallback re-read of the served page; /events pushes each change as it happens
TASK_BASE = "origin/main"  # task worktrees and the board branch start here; the server fetches it every minute


@dataclass
class Task:
    id: str
    title: str
    status: str
    milestone: str
    pr: int | None
    body: str
    file: str
    worktree: str
    issue: int | None = None  # optional GitHub issue number


@dataclass
class WorktreeInfo:
    name: str
    branch: str
    last_commit: str
    dirty: int
    ahead: int
    idle_min: int  # minutes since the last change (dirty file or commit)
    agent: bool
    remote: bool = False  # a pushed branch with no local worktree
    unique: int = 0  # commits on no other local or remote branch: what removing the branch loses


@dataclass
class AgentInfo:
    machine: str  # where the agent runs: this computer's name or a Herdr machine label
    machine_id: str | None  # Herdr machine id, None for this computer
    status: str  # Herdr: idle | working | blocked | done | unknown
    pane_id: str


def parse_task(text, file, worktree):
    """Parse one task file. Raises ValueError on a malformed file."""
    if not text.startswith("---\n"):
        raise ValueError(f"{file}: missing frontmatter")
    end = text.find("\n---\n", 4)
    if end == -1:
        raise ValueError(f"{file}: unterminated frontmatter")
    meta = {}
    for line in text[4:end].splitlines():
        if line.strip():
            key, sep, value = line.partition(":")
            if not sep:
                raise ValueError(f"{file}: bad frontmatter line {line!r}")
            meta[key.strip()] = value.strip()
    missing = [f for f in FIELDS if f not in meta]
    if missing:
        raise ValueError(f"{file}: missing {', '.join(missing)}")
    pr, issue = (number(meta.get(k, "null"), k, file) for k in ("pr", "issue"))
    return Task(
        id=meta["id"],
        title=meta["title"],
        status=meta["status"],
        milestone=meta["milestone"],
        pr=pr,
        body=text[end + 5 :].lstrip("\n"),
        file=file,
        worktree=worktree,
        issue=issue,
    )


def number(value, key, file):
    if value in ("", "null"):
        return None
    if value.isdigit():
        return int(value)
    raise ValueError(f"{file}: {key} must be a number or null, got {value!r}")


def parse_milestones(text):
    """Milestone names from tasks/milestones.md: one "- name" list item each. Pure."""
    return [line[2:].strip() for line in text.splitlines() if line.startswith("- ") and line[2:].strip()]


def add_milestone(text, name):
    """tasks/milestones.md with one more milestone. Pure."""
    if not MILESTONE_RE.match(name):
        raise ValueError(f"milestone {name!r}: use letters, digits, '.', '_' or '-' (max 32)")
    if name in parse_milestones(text):
        raise ValueError(f"milestone {name} already exists")
    return text.rstrip("\n") + f"\n- {name}\n"


def add_artifact(entries, path, label, at):
    """The artifact list with path on top (once), capped at MAX_ARTIFACTS. Pure."""
    rest = [e for e in entries if e["path"] != path]
    return [{"path": path, "label": label, "at": at}, *rest][:MAX_ARTIFACTS]


def edit_location(task):
    """Where an edit to this task is written: the board branch, or the task's own worktree. Pure.

    Editing a started task on its own branch keeps the edit next to the agent's
    status changes, so the two never conflict on merge.
    """
    if task.worktree in (MAIN, BOARD):
        return BOARD
    if task.worktree.startswith("origin/"):
        raise ValueError(f"{task.id} is checked out elsewhere ({task.worktree}); edit it there")
    return task.worktree


def set_field(text, key, value):
    """The task file text with one frontmatter field replaced. Pure."""
    end = text.find("\n---\n", 4)
    head, n = re.subn(rf"^{re.escape(key)}:.*$", f"{key}: {value}", text[:end], count=1, flags=re.M)
    if not n:
        raise ValueError(f"no {key} field")
    return head + text[end:]


def prune_done(tasks):
    """Files of the done tasks, which a release removes. Pure."""
    return [t.file for t in tasks if t.status == "done"]


NEW_BODY = "## Context\n\nTo be written.\n\n## Acceptance\n\n- To be defined when the task starts.\n"


def slugify(title):
    """File-name slug of a title: ASCII words joined by '-', at most 40 chars. Pure."""
    ascii_title = unicodedata.normalize("NFKD", title).encode("ascii", "ignore").decode()
    return re.sub(r"[^a-z0-9]+", "-", ascii_title.lower()).strip("-")[:40].rstrip("-")


def next_id(ids):
    """The id after the highest one ever used, so a deleted task's id is never reused. Pure."""
    return f"T-{max((int(i[2:]) for i in ids), default=0) + 1:04d}"


def new_task_text(task_id, title, milestone, body):
    """The file of a new todo task. Pure."""
    title = title.strip()
    if not title or "\n" in title or "\r" in title or len(title) > 120:
        raise ValueError("the title must be one line of 1 to 120 characters")
    body = body.replace("\r\n", "\n").strip("\n")
    body = body + "\n" if body else NEW_BODY
    return f"---\nid: {task_id}\ntitle: {title}\nstatus: todo\nmilestone: {milestone}\npr: null\n---\n\n{body}"


LIST_RE = re.compile(r"^\s*(?:([-*])|\d+\.)\s+(.*)$")
HEADING_RE = re.compile(r"^(#{1,6})\s+(.*)$")


def inline_markdown(text):
    """Code spans, links (http/https only), bold and italic of one escaped line. Pure."""
    out = []
    for part in re.split(r"(`[^`]+`)", text):
        if len(part) > 1 and part.startswith("`") and part.endswith("`"):
            out.append(f"<code>{html.escape(part[1:-1])}</code>")
            continue
        part = html.escape(part)
        part = re.sub(r"\[([^\]]+)\]\((https?://[^)\s]+)\)", r'<a href="\2" target="_blank" rel="noopener">\1</a>', part)
        part = re.sub(r"\*\*(.+?)\*\*", r"<strong>\1</strong>", part)
        part = re.sub(r"(?<![*\w])\*(?!\s)(.+?)(?<!\s)\*(?![*\w])", r"<em>\1</em>", part)
        out.append(part)
    return "".join(out)


def render_markdown(text):
    """The subset of Markdown task files use (headings, paragraphs, lists, code) as escaped HTML. Pure."""
    out, para, items, ordered = [], [], [], False

    def flush():
        nonlocal para, items
        if para:
            out.append(f"<p>{inline_markdown(chr(10).join(para))}</p>")
        if items:
            tag = "ol" if ordered else "ul"
            out.append(f"<{tag}>" + "".join(f"<li>{inline_markdown(i)}</li>" for i in items) + f"</{tag}>")
        para, items = [], []

    lines = text.replace("\r\n", "\n").split("\n")
    i = 0
    while i < len(lines):
        line = lines[i]
        i += 1
        if line.startswith("```"):
            flush()
            code = []
            while i < len(lines) and not lines[i].startswith("```"):
                code.append(lines[i])
                i += 1
            i += 1
            out.append(f"<pre><code>{html.escape(chr(10).join(code))}</code></pre>")
        elif m := HEADING_RE.match(line):
            flush()
            level = min(len(m.group(1)) + 1, 6)
            out.append(f"<h{level}>{inline_markdown(m.group(2))}</h{level}>")
        elif m := LIST_RE.match(line):
            if para or (items and ordered != (m.group(1) is None)):
                flush()
            ordered = m.group(1) is None
            items.append(m.group(2))
        elif not line.strip():
            flush()
        elif items and line.startswith(" "):
            items[-1] += " " + line.strip()
        else:
            if items:
                flush()
            para.append(line)
    flush()
    return "".join(out)


def excerpt(body):
    """The first two lines of text of a task body, without Markdown markup. Pure."""
    lines = []
    for line in body.splitlines():
        line = line.strip()
        if not line or line.startswith(("#", "```")):
            continue
        if m := LIST_RE.match(line):
            line = m.group(2)
        line = re.sub(r"\[([^\]]+)\]\([^)]*\)", r"\1", line)
        lines.append(re.sub(r"[`*]", "", line))
        if len(lines) == 2:
            break
    return " ".join(lines)


def milestone_key(name):
    """Sort key: versions (0.7.0 < v0.10.0) in numeric order, then named milestones (later). Pure."""
    if re.fullmatch(r"v?\d+(?:\.\d+)*", name):
        return (0, tuple(int(n) for n in name.lstrip("v").split(".")), "")
    return (1, (), name)


def validate(tasks, milestones=None):
    """Errors for one checkout's tasks; milestones, when given, is the allowed list. Pure."""
    errors = []
    seen = set()
    for t in tasks:
        if not ID_RE.match(t.id):
            errors.append(f"{t.file}: id {t.id!r} is not T-NNNN")
        if not t.file.startswith(t.id + "-"):
            errors.append(f"{t.file}: file name must start with {t.id}-")
        if t.status not in STATUSES:
            errors.append(f"{t.file}: status {t.status!r} is not one of {', '.join(STATUSES)}")
        if t.status in ("review", "done") and t.pr is None:
            errors.append(f"{t.file}: status {t.status} needs a pr number")
        if milestones is not None and t.milestone not in milestones:
            errors.append(f"{t.file}: milestone {t.milestone!r} is not in tasks/milestones.md")
        if t.id in seen:
            errors.append(f"{t.file}: duplicate id {t.id}")
        seen.add(t.id)
    return errors


def merge(tasks, deleted=()):
    """One task per id. Pure.

    The most advanced status wins; on a tie the task's own worktree beats the
    board branch, which beats the base. A task deleted on the board is dropped
    from every source, its own worktree and pushed branch included.
    """

    def key(t):
        return RANK.get(t.status, -1), {MAIN: 0, BOARD: 1}.get(t.worktree, 2)

    best = {}
    for t in tasks:
        if t.id in deleted:
            continue
        cur = best.get(t.id)
        if cur is None or key(t) > key(cur):
            best[t.id] = t
    return [best[k] for k in sorted(best)]


def is_active(idle_min, has_process):
    """A claude process in the worktree, or a change in the last few minutes. Pure.

    The process check alone misses most agents: a session keeps the cwd it was
    launched in, so one started from the main checkout that then created its
    worktree still reports the main checkout.
    """
    return has_process or idle_min < ACTIVE_MINUTES


def task_of_path(path):
    """The task id of a path inside a task worktree (.claude/worktrees/T-NNNN-slug), else None. Pure."""
    m = TASK_PATH_RE.search(path)
    return m.group(1) if m else None


def loss_text(wt):
    """What discarding this worktree throws away, for the confirmation. Pure."""
    parts = []
    if wt.dirty:
        parts.append(f"{wt.dirty} changed file{'s' if wt.dirty != 1 else ''}")
    if wt.unique:
        parts.append(f"{wt.unique} commit{'s' if wt.unique != 1 else ''} on no other branch")
    return " and ".join(parts) or "nothing unsaved"


def discard_button(task_id, wt):
    return f'<button class="btn danger" data-discard="{html.escape(task_id)}" data-loss="{html.escape(loss_text(wt))}">Discard worktree</button>'


def needs_you(status):
    return status in NEEDS_YOU


def agents_by_task(entries):
    """Task id -> AgentInfo from (machine label, machine id, Herdr agent) entries. Pure.

    An agent belongs to the task whose worktree is its working directory; when
    two share a task, the one waiting for the user wins, then a working one.
    """
    order = {"blocked": 4, "done": 3, "working": 2, "idle": 1}
    found = {}
    for machine, machine_id, a in entries:
        task_id = task_of_path(a.get("foreground_cwd") or a.get("cwd") or "")
        if task_id is None:
            continue
        info = AgentInfo(machine=machine, machine_id=machine_id, status=a.get("agent_status", "unknown"), pane_id=a["pane_id"])
        cur = found.get(task_id)
        if cur is None or order.get(info.status, 0) > order.get(cur.status, 0):
            found[task_id] = info
    return found


OPEN = ("todo", "doing", "review")  # what `task_board.py list` shows without --status/--all


def task_rows(tasks, worktrees, agents):
    """One JSON-ready dict per task, for the CLI (list, show, summary). Pure."""
    rows = []
    for t in tasks:
        wt = worktrees.get(t.worktree) if t.worktree not in (MAIN, BOARD) else None
        ag = agents.get(t.id)
        rows.append(
            {
                "id": t.id,
                "title": t.title,
                "status": t.status,
                "milestone": t.milestone,
                "pr": t.pr,
                "issue": t.issue,
                "file": t.file,
                "branch": wt.branch if wt else None,
                "where": ("remote" if wt.remote else "local") if wt else None,
                "last_commit": wt.last_commit if wt else None,
                "dirty": wt.dirty if wt else 0,
                "ahead": wt.ahead if wt else 0,
                "idle_min": wt.idle_min if wt else None,
                "agent": ag.status if ag else None,
                "machine": ag.machine if ag else None,
                "needs_you": bool(ag and needs_you(ag.status)),
            }
        )
    return rows


def filter_rows(rows, statuses=OPEN, milestone=None, needs_you=False):
    """The rows a CLI listing keeps. Pure."""
    return [
        r
        for r in rows
        if r["status"] in statuses and (milestone is None or r["milestone"] == milestone) and (not needs_you or r["needs_you"])
    ]


def agent_label(row):
    if row["agent"] is None:
        return "-"
    state = "needs you" if row["needs_you"] else row["agent"]
    return f"{state} ({row['machine']})"


def format_rows(rows):
    """An aligned plain-text table of rows, sorted like the board. Pure."""
    if not rows:
        return "no tasks"
    rows = sorted(rows, key=lambda r: (RANK.get(r["status"], -1), milestone_key(r["milestone"]), r["id"]))
    table = [("ID", "STATUS", "MILESTONE", "AGENT", "TITLE")] + [(r["id"], r["status"], r["milestone"], agent_label(r), r["title"]) for r in rows]
    widths = [max(len(line[i]) for line in table) for i in range(4)]
    return "\n".join("  ".join(cell.ljust(w) for cell, w in zip(line[:4], widths)) + "  " + line[4] for line in table)


def board_summary(rows):
    """Counts of the open statuses, the tasks whose agent waits, and a one-line text. Pure."""
    counts = {s: sum(1 for r in rows if r["status"] == s) for s in OPEN}
    waiting = [r["id"] for r in rows if r["needs_you"]]
    text = "tasks: " + " · ".join(f"{n} {s}" for s, n in counts.items() if n) if any(counts.values()) else "tasks: none open"
    if waiting:
        text += " · needs you: " + ", ".join(waiting)
    return {"counts": counts, "needs_you": waiting, "text": text}


def page_version(page, generated):
    """A fingerprint of the board page that ignores its "Generated" stamp. Pure."""
    return hashlib.sha1(page.replace(generated, "").encode()).hexdigest()[:16]


def render_html(tasks, worktrees, repo, generated, token, warnings=(), milestones=(), agents=None, artifacts=None):
    """The board page task_board.py serves; token guards the requests it sends back. Pure."""
    e = html.escape
    milestones = list(milestones) + sorted({t.milestone for t in tasks} - set(milestones))
    agents = agents or {}
    artifacts = artifacts or {}

    def artifact_href(task_id, i, path):
        # Through the board: a page on http:// cannot open file://.
        return f"/a/{task_id}/{i}/{quote(Path(path).name)}"
    columns = []
    for status in STATUSES:
        cards = []
        for t in sorted((t for t in tasks if t.status == status), key=lambda t: (milestone_key(t.milestone), t.id)):
            wt = worktrees.get(t.worktree)
            choices = "".join(f'<option value="{e(m)}"{" selected" if m == t.milestone else ""}>{e(m)}</option>' for m in milestones)
            meta = [f'<select class="msel" data-task="{e(t.id)}">{choices}</select>']
            if t.pr is not None:
                meta.append(f'<a class="pill" href="https://github.com/{e(repo)}/pull/{t.pr}" target="_blank" rel="noopener">#{t.pr}</a>')
            if t.issue is not None:
                meta.append(f'<a class="pill" href="https://github.com/{e(repo)}/issues/{t.issue}" target="_blank" rel="noopener">issue #{t.issue}</a>')
            ag = agents.get(t.id)
            live = ""
            if ag is not None:
                state = '<span class="agent">● agent active</span>' if ag.status == "working" else f'<span class="state">{e(ag.status)}</span>'
                chips = [f'<span class="pill">on {e(ag.machine)}</span>', state]
                if needs_you(ag.status):
                    chips.append(f'<button class="chip needs" data-focus="{e(t.id)}">needs you</button>')
                live = f'<div class="live">{"".join(chips)}</div>'
            arts = artifacts.get(t.id, [])
            links = "".join(
                f'<li><a href="{e(artifact_href(t.id, i, a["path"]))}" target="_blank" rel="noopener">{e(a["label"])}</a> <span class="state">{e(a["at"])}</span></li>'
                for i, a in enumerate(arts)
            )
            if arts:
                meta.append(f'<a class="pill" href="{e(artifact_href(t.id, 0, arts[0]["path"]))}" target="_blank" rel="noopener">{e(arts[0]["label"])}</a>')
            activity = ""
            if wt is not None and t.worktree not in (MAIN, BOARD) and t.status in ("doing", "review"):
                if wt.remote:
                    activity = (
                        f'<div class="wt"><span class="pill">remote branch (another machine or the cloud)</span><code>{e(wt.branch)}</code>'
                        f"<span>{e(wt.last_commit)}</span><span>{wt.ahead} ahead of origin/main</span></div>"
                    )
                else:
                    agent = '<span class="agent">● agent active</span>' if wt.agent and ag is None else ""
                    activity = (
                        f'<div class="wt">{agent}<code>{e(wt.branch)}</code>'
                        f"<span>{e(wt.last_commit)}</span>"
                        f"<span>{wt.dirty} changed · {wt.ahead} ahead of origin/main · last change {wt.idle_min} min ago</span></div>"
                    )
            command = f'claude "/work-on-task {t.id}"'
            text = excerpt(t.body)
            summary = f'<p class="excerpt">{e(text)}</p>' if text else ""
            delete = f'<button class="btn danger" data-delete="{e(t.id)}">Delete</button>'
            if wt is not None and not wt.remote and t.worktree not in (MAIN, BOARD):
                delete += discard_button(t.id, wt)
            launch = f'<button class="btn primary" data-launch="{e(t.id)}">Open in Herdr</button>'
            has_agent = ' data-agent="1"' if ag else ""
            cards.append(
                f'<article class="card" id="{e(t.id)}" data-milestone="{e(t.milestone)}"{has_agent} '
                f'data-text="{e((t.id + " " + t.title + " " + t.body).lower())}">'
                f'<header><span class="tid">{e(t.id)}</span>{"".join(meta)}</header>'
                f'<h3>{e(t.title)}</h3>{summary}{live}{activity}'
                f'<div class="actions">{launch}'
                f'<button class="btn" data-copy="{e(command)}">Copy command</button>'
                f'<button class="btn" data-open="{e(t.id)}">Details</button>'
                f'{delete}</div>'
                f'<div class="body" hidden>{render_markdown(t.body)}</div><ul class="arts" hidden>{links}</ul></article>'
            )
        columns.append(
            f'<section class="col" data-status="{status}"><h2>{status} <span class="count">{len(cards)}</span></h2>'
            f'{"".join(cards) or "<p class=empty>—</p>"}</section>'
        )
    options = "".join(f'<option value="{e(m)}">{e(m)}</option>' for m in milestones)
    notes = "".join(f'<p class="warn">{e(w)}</p>' for w in warnings)
    ids = {t.id for t in tasks}
    for wt in worktrees.values():
        m = TASK_BRANCH_RE.match(wt.branch)
        if not wt.remote and m and m.group(1) not in ids:
            notes += f'<p class="warn">{e(m.group(1))} was deleted; its worktree <code>{e(wt.branch)}</code> is still here. {discard_button(m.group(1), wt)}</p>'

    return PAGE.format(
        generated=e(generated),
        options=options,
        new_panel=NEW_PANEL.format(options=options),
        notes=notes,
        columns="".join(columns),
        token=json.dumps(token),
        refresh=SERVE_JS % (POLL_SECONDS * 1000),
    )


# Swap in a fresh board without losing scroll, filter or the open panel. The
# server announces each change on /events, so a status shows within a second
# or two; the timer is only the fallback for a dropped event stream.
SERVE_JS = """let pending = false;
async function refresh() {
  // Not while a control the swap replaces (a card's milestone picker) has focus, nor while
  // a delete or discard waits for its confirming click: retry once that ends.
  const held = document.activeElement && document.activeElement.closest(".board, #notes") && document.activeElement.matches("input, select, textarea");
  if (held || document.querySelector("[data-armed]")) { pending = true; return; }
  pending = false;
  try {
    const r = await fetch("/?token=" + TOKEN, { cache: "no-store" });
    if (!r.ok) throw new Error(r.status);
    const doc = new DOMParser().parseFromString(await r.text(), "text/html");
    for (const sel of [".board", "#notes", ".gen"]) document.querySelector(sel).innerHTML = doc.querySelector(sel).innerHTML;
    apply();
    if (!panel.hidden) openTask(readHash().open);
  } catch (err) { document.querySelector(".gen").textContent = "Board server unreachable (" + err.message + ")"; }
}
document.addEventListener("focusout", () => setTimeout(() => { if (pending) refresh(); }, 0));
setInterval(() => { if (pending) refresh(); }, 1000);  // an armed button disarms on its own timer
let version = null;
const events = new EventSource("/events?token=" + TOKEN);
events.onmessage = ev => { if (ev.data !== version) { version = ev.data; refresh(); } };
setInterval(refresh, %d);"""


NEW_PANEL = """<aside id="npanel" class="side" hidden><button class="btn" id="nclose">Close</button><h2>New task</h2>
<div class="form"><label>Title<input id="ntitle" maxlength="120"></label>
<label>Milestone<select id="nms">{options}</select></label>
<label>Details (Markdown; empty for the Context/Acceptance template)<textarea id="nbody"></textarea></label>
<div class="actions"><button class="btn primary" id="ncreate">Create</button></div></div></aside>"""


PAGE = """<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>EmailOps Tasks</title>
<style>
:root {{ --bg:#f6f7f9; --panel:#fff; --text:#1c2024; --muted:#646b75; --line:#dfe3e8; --accent:#2f6fde; --ok:#1f8a4c; --warn:#b26b00; }}
@media (prefers-color-scheme: dark) {{ :root {{ --bg:#121417; --panel:#1b1e23; --text:#e6e8eb; --muted:#9aa1ab; --line:#2c3138; --accent:#6b9cf5; --ok:#4cc27e; --warn:#e3a33b; }} }}
* {{ box-sizing:border-box; }}
body {{ margin:0; background:var(--bg); color:var(--text); font:14px/1.45 -apple-system, system-ui, sans-serif; }}
.top {{ display:flex; flex-wrap:wrap; gap:8px 16px; align-items:center; padding:12px 16px; border-bottom:1px solid var(--line); background:var(--panel); position:sticky; top:0; z-index:2; }}
.top h1 {{ font-size:16px; margin:0; }}
.top .gen {{ color:var(--muted); margin-left:auto; }}
input, select {{ font:inherit; padding:5px 8px; border:1px solid var(--line); border-radius:6px; background:var(--bg); color:var(--text); }}
.warn {{ margin:8px 16px 0; color:var(--warn); }}
.board {{ display:grid; grid-template-columns:minmax(360px, 2fr) repeat(4, minmax(180px, 1fr)); gap:12px; padding:16px; overflow-x:auto; }}
@media (max-width: 760px) {{ .board {{ grid-template-columns:1fr; }} }}
.col h2 {{ font-size:12px; text-transform:uppercase; letter-spacing:.06em; color:var(--muted); margin:0 0 8px; }}
.count {{ font-weight:400; }}
.card {{ background:var(--panel); border:1px solid var(--line); border-radius:8px; padding:10px; margin-bottom:10px; cursor:pointer; }}
.card:hover {{ border-color:var(--accent); }}
.excerpt {{ margin:0 0 8px; color:var(--muted); font-size:13px; display:-webkit-box; -webkit-line-clamp:2; -webkit-box-orient:vertical; overflow:hidden; }}
.card header {{ display:flex; gap:6px; align-items:center; flex-wrap:wrap; }}
.tid {{ font:12px ui-monospace, monospace; color:var(--muted); }}
.pill {{ font-size:11px; padding:1px 6px; border:1px solid var(--line); border-radius:999px; color:var(--muted); text-decoration:none; }}
.card h3 {{ font-size:14px; margin:6px 0; }}
.wt {{ display:flex; flex-direction:column; gap:2px; font-size:12px; color:var(--muted); margin-bottom:6px; }}
.wt code {{ color:var(--text); }}
.agent {{ color:var(--ok); font-weight:600; }}
.live {{ display:flex; gap:6px; align-items:center; flex-wrap:wrap; font-size:12px; margin-bottom:6px; }}
.state {{ color:var(--muted); }}
.btn.danger {{ color:#c0392b; }}
.msel {{ font-size:11px; padding:1px 4px; border-radius:999px; }}
.chip.needs {{ font:600 11px inherit; padding:1px 8px; border-radius:999px; border:1px solid #d4a800; background:#f5c518; color:#1c2024; cursor:pointer; }}
.actions {{ display:flex; gap:6px; flex-wrap:wrap; }}
.btn {{ font:12px inherit; padding:4px 8px; border:1px solid var(--line); border-radius:6px; background:var(--bg); color:var(--text); cursor:pointer; text-decoration:none; }}
.btn.primary {{ background:var(--accent); border-color:var(--accent); color:#fff; }}
.empty {{ color:var(--muted); }}
.hidden {{ display:none; }}
.side {{ position:fixed; top:0; right:0; bottom:0; width:min(560px, 100%); background:var(--panel); border-left:1px solid var(--line); padding:16px; overflow:auto; z-index:3; box-shadow:-8px 0 24px rgba(0,0,0,.15); }}
.side pre {{ white-space:pre-wrap; font:12px/1.5 ui-monospace, monospace; }}
.md {{ font-size:14px; line-height:1.55; }}
.md p {{ white-space:normal; }}
.md code {{ font:12px ui-monospace, monospace; background:var(--bg); padding:1px 4px; border-radius:4px; }}
.md pre {{ background:var(--bg); padding:8px; border-radius:6px; overflow:auto; }}
.md pre code {{ padding:0; }}
.md h3 {{ font-size:15px; margin:18px 0 6px; }}
.md h4, .md h5, .md h6 {{ font-size:14px; margin:14px 0 4px; }}
.md a {{ color:var(--accent); }}
#poutd summary {{ cursor:pointer; font-weight:600; margin-top:16px; }}
.form {{ display:flex; flex-direction:column; gap:10px; }}
.form label {{ display:flex; flex-direction:column; gap:4px; font-size:12px; color:var(--muted); }}
textarea {{ font:13px/1.5 ui-monospace, monospace; padding:6px 8px; border:1px solid var(--line); border-radius:6px; background:var(--bg); color:var(--text); min-height:220px; }}
</style></head>
<body>
<div class="top"><h1>Tasks</h1>
<select id="ms"><option value="">All milestones</option>{options}</select>
<input id="q" type="search" placeholder="Search">
<button class="btn primary" id="newtask">New task</button>
<input id="newms" placeholder="New milestone" maxlength="32"><button class="btn" id="addms">Add</button>
<span class="gen">Generated {generated}</span></div>
<div id="notes">{notes}</div>
<main class="board">{columns}</main>
<aside id="panel" class="side" hidden><button class="btn" id="close">Close</button><h2 id="ptitle"></h2><div id="pbody" class="md"></div>
<h3 id="partsh" hidden>Artifacts</h3><ul id="parts" hidden></ul>
<details id="poutd"><summary>Agent output</summary><pre id="pout"></pre></details></aside>
{new_panel}
<script>
const TOKEN = {token};
const ms = document.getElementById("ms"), q = document.getElementById("q"), panel = document.getElementById("panel");
function readHash() {{ const p = new URLSearchParams(location.hash.slice(1)); return {{ ms: p.get("ms") || "", q: p.get("q") || "", open: p.get("open") || "" }}; }}
function writeHash(open) {{ const p = new URLSearchParams(); if (ms.value) p.set("ms", ms.value); if (q.value) p.set("q", q.value); if (open) p.set("open", open); history.replaceState(null, "", "#" + p); }}
function apply() {{
  const term = q.value.toLowerCase();
  document.querySelectorAll(".col").forEach(col => {{
    let n = 0;
    col.querySelectorAll(".card").forEach(c => {{
      const show = (!ms.value || c.dataset.milestone === ms.value) && (!term || c.dataset.text.includes(term));
      c.classList.toggle("hidden", !show); if (show) n++;
    }});
    col.querySelector(".count").textContent = n;
  }});
}}
const poutd = document.getElementById("poutd"), pout = document.getElementById("pout"), npanel = document.getElementById("npanel");
let shown = "";
function loadOutput() {{
  if (!poutd.open || poutd.hidden) return;
  fetch("/output?token=" + TOKEN + "&task=" + shown, {{ cache: "no-store" }}).then(r => r.text()).then(t => {{ pout.textContent = t; }}, () => {{ pout.textContent = "Agent output unavailable"; }});
}}
function openTask(id) {{
  const card = document.getElementById(id); if (!card) return;
  // Another task: start with its output collapsed. The same one (a refresh): keep it as the reader left it.
  if (id !== shown) {{ poutd.open = false; pout.textContent = ""; shown = id; }}
  document.getElementById("ptitle").textContent = id + " · " + card.querySelector("h3").textContent;
  document.getElementById("pbody").innerHTML = card.querySelector(".body").innerHTML;
  panel.hidden = false; npanel.hidden = true; writeHash(id);
  const parts = document.getElementById("parts"), arts = card.querySelector(".arts");
  parts.innerHTML = arts.innerHTML; parts.hidden = document.getElementById("partsh").hidden = !arts.children.length;
  poutd.hidden = !(TOKEN && card.dataset.agent);
  loadOutput();
}}
poutd.addEventListener("toggle", loadOutput);
function post(path, b, label, data) {{
  // A <select> reports through its title: setting its text would wipe its options.
  const show = text => {{ if (b.tagName !== "SELECT") b.textContent = text; }};
  show("…");
  return fetch(path + (path.includes("?") ? "&" : "?") + "token=" + TOKEN, {{ method: "POST", body: data && new URLSearchParams(data) }}).then(r => r.json()).then(j => {{
    show(j.ok ? "✓" : "Failed"); b.title = j.error || "";
    setTimeout(() => show(label), 2500);
    if (j.ok) refresh();
    return j;
  }}, () => {{ show("Failed"); b.title = "Board server unreachable"; }});
}}
const s = readHash(); ms.value = s.ms; q.value = s.q; apply(); if (s.open) openTask(s.open);
ms.onchange = q.oninput = () => {{ apply(); writeHash(panel.hidden ? "" : readHash().open); }};
document.addEventListener("change", ev => {{
  const s = ev.target.closest(".msel"); if (!s) return;
  s.blur();
  post("/milestone/set?task=" + s.dataset.task + "&name=" + encodeURIComponent(s.value), s, s.value);
}});
document.getElementById("close").onclick = () => {{ panel.hidden = true; shown = ""; writeHash(""); }};
document.addEventListener("click", ev => {{
  const b = ev.target.closest("button");
  if (!b) {{
    // A click on the card itself, not on one of its controls or links, opens its details.
    const card = ev.target.closest(".card");
    if (card && !ev.target.closest("a, select, input") && !getSelection().toString()) openTask(card.id);
    return;
  }}
  if (b.id === "newtask") {{ npanel.hidden = false; panel.hidden = true; shown = ""; writeHash(""); document.getElementById("ntitle").focus(); }}
  if (b.id === "nclose") npanel.hidden = true;
  if (b.id === "ncreate") {{
    const title = document.getElementById("ntitle"), body = document.getElementById("nbody");
    post("/task/new", b, "Create", {{ title: title.value, milestone: document.getElementById("nms").value, body: body.value }}).then(j => {{
      if (j && j.ok) {{ title.value = body.value = ""; npanel.hidden = true; }}
    }});
  }}
  if (b.dataset.open) openTask(b.dataset.open);
  if (b.dataset.launch) post("/open?task=" + b.dataset.launch, b, "Open in Herdr");
  if (b.dataset.focus) post("/focus?task=" + b.dataset.focus, b, "needs you");
  if (b.dataset.delete) {{
    // Two clicks: the first arms the button for a few seconds.
    if (!b.dataset.armed) {{ b.dataset.armed = "1"; b.textContent = "Confirm delete"; setTimeout(() => {{ delete b.dataset.armed; b.textContent = "Delete"; }}, 4000); }}
    else post("/delete?task=" + b.dataset.delete, b, "Delete");
  }}
  if (b.dataset.discard) {{
    // Removes the worktree with its uncommitted work and the local branch: the confirming click names what goes.
    if (!b.dataset.armed) {{ b.dataset.armed = "1"; b.textContent = "Discard " + b.dataset.loss + "?"; setTimeout(() => {{ delete b.dataset.armed; b.textContent = "Discard worktree"; }}, 5000); }}
    else {{ delete b.dataset.armed; post("/discard?task=" + b.dataset.discard, b, "Discard worktree"); }}
  }}
  if (b.id === "addms") {{
    const input = document.getElementById("newms");
    post("/milestone/new?name=" + encodeURIComponent(input.value.trim()), b, "Add").then(j => {{ if (j && j.ok) input.value = ""; }});
  }}
  if (b.dataset.copy) navigator.clipboard.writeText(b.dataset.copy).then(() => {{ b.textContent = "Copied"; setTimeout(() => b.textContent = "Copy command", 1500); }}, () => {{ b.textContent = "Copy failed"; }});
}});
{refresh}
</script></body></html>
"""


# --- I/O ------------------------------------------------------------------


def git(*args, cwd=None):
    return subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True, check=True).stdout


def main_root():
    return Path(git("rev-parse", "--path-format=absolute", "--git-common-dir").strip()).parent


def read_tasks(checkout, label, pattern="T-*.md"):
    """Tasks of one checkout (from disk) plus parse errors (as strings)."""
    tasks, errors = [], []
    for path in sorted((checkout / "tasks").glob(pattern)):
        try:
            tasks.append(parse_task(path.read_text(encoding="utf-8"), path.name, label))
        except ValueError as err:
            errors.append(str(err))
    return tasks, errors


def read_ref_tasks(ref, label, prefix="T-"):
    """Tasks committed on a git ref plus parse errors."""
    tasks, errors = [], []
    for name in git("ls-tree", "--name-only", ref, "tasks/").split():
        file = name.removeprefix("tasks/")
        if file.startswith(prefix) and file.endswith(".md"):
            try:
                tasks.append(parse_task(git("show", f"{ref}:{name}"), file, label))
            except ValueError as err:
                errors.append(str(err))
    return tasks, errors


def worktrees():
    """(path, branch) of every worktree; the main checkout first."""
    out, path, result = git("worktree", "list", "--porcelain"), None, []
    for line in out.splitlines() + [""]:
        if line.startswith("worktree "):
            path, branch = Path(line[9:]), "(detached)"
        elif line.startswith("branch "):
            branch = line[7:].removeprefix("refs/heads/")
        elif not line and path is not None:
            result.append((path, branch))
            path = None
    return result


def claude_cwds():
    """Working directories of running claude processes, by binary name."""
    proc = subprocess.run(["lsof", "-a", "-c", "claude", "-d", "cwd", "-Fn"], capture_output=True, text=True)
    # lsof exits 1 when nothing matches; that is "no agents", not a failure.
    if proc.returncode not in (0, 1):
        raise OSError(proc.stderr.strip() or f"lsof exited {proc.returncode}")
    return [Path(line[1:]) for line in proc.stdout.splitlines() if line.startswith("n")]


def owner_of(cwd, paths):
    """The deepest worktree path containing cwd (the main checkout contains the others)."""
    hits = [p for p in paths if cwd == p or p in cwd.parents]
    return max(hits, key=lambda p: len(p.parts), default=None)


def activity(path, branch, label, has_process, now):
    def out(*args):
        try:
            return git(*args, cwd=path)
        except subprocess.CalledProcessError:
            return ""

    status = out("status", "--porcelain").splitlines()
    ahead = out("rev-list", "--count", "origin/main..HEAD").strip()
    unique = out("rev-list", "--count", "HEAD", "--not", f"--exclude=refs/heads/{branch}", "--branches", "--remotes").strip()
    commit_time = out("log", "-1", "--format=%ct").strip()
    times = [int(commit_time)] if commit_time.isdigit() else []
    for line in status:
        # "XY path" or "XY old -> new"; a deleted file has no mtime.
        try:
            times.append((path / line[3:].split(" -> ")[-1].strip('"')).stat().st_mtime)
        except OSError:
            pass
    idle_min = int((now - max(times)) // 60) if times else 10**6
    return WorktreeInfo(
        name=label,
        branch=branch,
        last_commit=out("log", "-1", "--format=%cr · %s").strip(),
        dirty=len(status),
        ahead=int(ahead) if ahead.isdigit() else 0,
        idle_min=idle_min,
        agent=is_active(idle_min, has_process),
        unique=int(unique) if unique.isdigit() else 0,
    )


def read_artifacts(checkout):
    path = checkout / ARTIFACTS
    if not path.exists():
        return []
    # Only files that still exist: a report dir may have been cleaned since.
    return [a for a in json.loads(path.read_text(encoding="utf-8")) if Path(a["path"]).exists()]


def remote_activity(ref, now):
    """WorktreeInfo for a pushed task branch nobody has checked out here."""
    commit_time, summary = git("log", "-1", "--format=%ct%n%cr · %s", ref).split("\n", 1)
    return WorktreeInfo(
        name=ref,
        branch=ref.removeprefix("origin/"),
        last_commit=summary.strip(),
        dirty=0,
        ahead=int(git("rev-list", "--count", f"origin/main..{ref}").strip()),
        idle_min=int((now - int(commit_time)) // 60),
        agent=False,
        remote=True,
    )


def collect():
    """(main checkout, merged tasks, worktree infos, milestones, warnings, artifacts by task id) as they are right now."""
    root = main_root()
    now = time.time()
    tasks, warnings = [], []

    def add(found, label):
        tasks.extend(found[0])
        warnings.extend(f"{label}: {err}" for err in found[1])

    add(read_ref_tasks(TASK_BASE, MAIN), TASK_BASE)
    milestones = parse_milestones(git("show", f"{TASK_BASE}:tasks/milestones.md"))
    deleted = set()
    board = root / ".claude" / "worktrees" / BOARD
    if board.exists():
        base = git("merge-base", "HEAD", TASK_BASE, cwd=board).strip()
        for line in git("diff", "--name-status", base, "--", "tasks/", cwd=board).splitlines():
            change, name = line.split("\t")[0], line.split("\t")[-1]
            file = name.removeprefix("tasks/")
            if change == "D" and file.startswith("T-"):
                deleted.add(file[:6])
            elif file.startswith("T-"):
                add(read_tasks(board, BOARD, file), BOARD)
        milestones = read_milestones(board)

    trees = worktrees()
    try:
        cwds = claude_cwds()
    except OSError as err:
        warnings.append(f"lsof unavailable, live agents not shown: {err}")
        cwds = []
    busy = {owner_of(c, [p for p, _ in trees]) for c in cwds}
    infos, artifacts = {}, {}
    for path, branch in trees:
        m = TASK_BRANCH_RE.match(branch)
        if m:
            add(read_tasks(path, path.name, f"{m.group(1)}-*.md"), path.name)
            infos[path.name] = activity(path, branch, path.name, path in busy, now)
            artifacts[m.group(1)] = read_artifacts(path)
    local = {b for _, b in trees}
    refs = git("for-each-ref", "--format=%(refname:short)", *(f"refs/remotes/origin/{k}/T-*" for k in ("feature", "fix", "refactor")))
    for ref in refs.split():
        branch = ref.removeprefix("origin/")
        m = TASK_BRANCH_RE.match(branch)
        if m and branch not in local:
            add(read_ref_tasks(ref, ref, m.group(1)), ref)
            infos[ref] = remote_activity(ref, now)
    return root, merge(tasks, deleted), infos, milestones, warnings, artifacts


def repo_slug():
    url = git("remote", "get-url", "origin").strip()
    m = re.search(r"[:/]([^/:]+/[^/]+?)(?:\.git)?$", url)
    if not m:
        raise ValueError(f"cannot read owner/repo from origin {url!r}")
    return m.group(1)


def read_milestones(checkout):
    path = checkout / "tasks" / "milestones.md"
    return parse_milestones(path.read_text(encoding="utf-8")) if path.exists() else []


def cmd_check():
    tasks, errors = read_tasks(Path.cwd(), MAIN)
    errors += validate(tasks, read_milestones(Path.cwd()))
    for err in errors:
        print(err, file=sys.stderr)
    print(f"{len(tasks)} tasks, {len(errors)} errors")
    return 1 if errors else 0


def build_page(token, agents=None, warnings=()):
    """(page, version) built from collect(); the version ignores the "Generated" stamp."""
    _, tasks, infos, milestones, found_warnings, artifacts = collect()
    stamp = datetime.now().strftime("%d/%m/%Y %H:%M:%S")
    page = render_html(
        tasks, infos, repo_slug(), stamp, token, warnings=[*warnings, *found_warnings], milestones=milestones, agents=agents, artifacts=artifacts
    )
    return page, page_version(page, stamp)


def cmd_artifact(path, label):
    checkout = Path(git("rev-parse", "--show-toplevel").strip())
    if not TASK_BRANCH_RE.match(git("branch", "--show-current").strip()):
        raise ValueError("run it from a task worktree (branch feature/T-NNNN-...)")
    file = Path(path).resolve()
    if not file.is_file():
        raise ValueError(f"no such file: {file}")
    manifest = checkout / ARTIFACTS
    entries = json.loads(manifest.read_text(encoding="utf-8")) if manifest.exists() else []
    entries = add_artifact(entries, str(file), label or file.name, datetime.now().strftime("%d/%m/%Y %H:%M"))
    manifest.parent.mkdir(exist_ok=True)
    manifest.write_text(json.dumps(entries, indent=2) + "\n", encoding="utf-8")
    print(f"recorded {file.name} ({len(entries)} artifacts)")
    return 0


def cmd_prune_done():
    """git rm the done tasks of this checkout (the release skill runs it before its commit)."""
    tasks, errors = read_tasks(Path.cwd(), MAIN)
    if errors:
        raise ValueError("; ".join(errors))
    files = prune_done(tasks)
    for f in files:
        git("rm", "-q", f"tasks/{f}")
        print(f"removed tasks/{f}")
    print(f"{len(files)} done tasks pruned")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("check")
    sub.add_parser("prune-done")
    art_p = sub.add_parser("artifact")
    art_p.add_argument("path")
    art_p.add_argument("--label")
    args = parser.parse_args()
    if args.cmd == "prune-done":
        return cmd_prune_done()
    if args.cmd == "artifact":
        return cmd_artifact(args.path, args.label)
    return cmd_check()


if __name__ == "__main__":
    sys.exit(main())
