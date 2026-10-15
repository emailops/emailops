# Tasks

The roadmap's work items, one Markdown file per task. These files are the
source of truth for what is planned, in progress and done; `ROADMAP.md` is the
prose overview.

## Format

One file per task, named after its id and a short slug (for example
`T-0001-short-imperative-title.md`):

```markdown
---
id: T-0001
title: Short imperative title
status: todo
milestone: 0.7.0
pr: null
issue: null
---

## Context

Why the task exists and what it touches.

## Acceptance

- What must be true when it is done.
```

- `status`: `todo` → `doing` → `review` → `done`, or `dropped`.
- `review` and `done` need `pr` (the PR number).
- `milestone`: one of the names listed in `tasks/milestones.md`. The board lists each
  column from the lowest version up (`0.7.0` before `0.10.0`), named
  milestones such as `later` last.
- `issue` (optional): the GitHub issue the task comes from; the board links it.
- Done tasks are removed by the release (`tasks.py prune-done`, release
  skill Phase 3c): git history keeps them.
- New task: the id after the highest one any branch ever used, `status: todo`.
- Work on a task in its own worktree and only edit that task's file there, so
  parallel branches never conflict. `/work-on-task T-NNNN` does this.

## Commands

- `make tasks-check`: validate the files (also runs in the pre-commit hook).
- `make tasks-serve` (`scripts/task_board.py`): the task board, served live on
  `127.0.0.1:8780` (the printed URL carries a random token; leave it running in
  a terminal or Herdr pane). While the page is open the server re-reads the
  tasks and the agents Herdr runs here and on its saved machines every 1.5
  seconds and pushes each change to the page, which updates in place within a
  second or two (filters and the search box never hold it back). A card with an agent says where it runs; a yellow
  **needs you** chip (agent blocked on a question, or done with its turn)
  focuses that Herdr pane and raises the terminal. Each card shows the first
  two lines of its details; clicking the card (or **Details**) opens them
  rendered as Markdown, with the agent's latest output in a collapsed section. **Open in Herdr** focuses the task's agent if there is
  one, otherwise it creates (or reuses) the task's worktree
  `.claude/worktrees/T-NNNN-slug` on `feature/T-NNNN-slug`, opens a Herdr tab
  there, starts Claude and types `/work-on-task T-NNNN` (not sent). A new
  task branch starts from `origin/main` with `chore/tasks-board` merged on
  top, so it carries its task file and the milestones, and merging its PR
  (with a merge commit) brings the board's edits to `main`.
  The served board also edits: **New task** (title, milestone and Markdown
  details), **New milestone** (appended to `tasks/milestones.md`), a milestone
  picker on each card, and **Delete** (two clicks). Deleting a started task
  hides it from every source but keeps its worktree, branch and agent.
  **Discard worktree** (on a card with a worktree here, or next to the note
  about a deleted task whose worktree remains) removes everything of the task
  on this machine: the card, the Herdr panes inside the worktree, the worktree
  with its uncommitted work, and the local branch. The confirming click names
  what is lost (changed files, commits on no other branch). A pushed branch
  and its PR stay: delete them on GitHub. `make disk-clean ARGS="--apply"`
  frees the build directory the worktree leaves behind. Each edit is one commit on
  `chore/tasks-board` (worktree `.claude/worktrees/tasks-board`, created on
  first use), or on the task's own branch once it is started; a refused commit
  (pre-commit hook) leaves the file as it was. Nothing is pushed: merge
  `chore/tasks-board` through a PR (with a merge commit; the release skill
  offers it in Phase 1a) when you want the edits on `main`. Once its edits are
  merged, the board fast-forwards the branch to the base before its next edit,
  so it never drifts.
- From Claude: "create a task for …" (the `new-task` skill) drafts the title,
  context and acceptance and commits the task on `chore/tasks-board` through
  `python3 scripts/task_board.py new --title … --milestone … --body-file …`.
- From Claude, everything else the board does: the `tasks` skill (`/tasks`,
  "qué tareas hay", "empieza la T-0004 en segundo plano") runs
  `python3 scripts/task_board.py list | show | summary | open | focus |
  milestone | new-milestone | delete | discard`, the same code as the
  buttons. `close-merged <PR>...` sets done, on the board branch, the open
  tasks whose PR merged; `babysit-prs` runs it on every pass. `open --mode bg` starts the task as a background Claude session in
  its worktree (`claude attach T-NNNN`); `open --mode cloud` starts a cloud
  session in a Herdr tab, which the board shows once it pushes its branch.
- In Claude Code's status line: the `task-status` plugin
  (`.claude/plugins/task-status`) shows the open tasks by status every 20
  seconds and a toast when a task's agent starts waiting for you. Load it with
  `claude --plugin-dir .claude/plugins/task-status`, or for every session add
  its absolute path to `CLAUDE_CODE_PLUGIN_DIRS` in the `env` block of
  `~/.claude/settings.json`. `claude plugin test .claude/plugins/task-status`
  runs its tests.
- `python3 scripts/tasks.py artifact <path> --label "eval report"` (from a
  task worktree; `/work-on-task` does it) records a file the session produced.
  The card links the latest one and **Details** lists them; the live board
  serves them (and the files beside them, so an HTML report keeps its assets).
  The list is `.claude/task-artifacts.local.json` in that worktree, gitignored.
- What the board shows per task: the base branch's file, overridden by the
  board branch `chore/tasks-board` (edits made from the board), by the task's
  own worktree (uncommitted edits included) and by a pushed
  `feature/T-NNNN-*` branch with no local worktree (another machine or a
  cloud session); the most advanced status wins, and a deletion on the board
  branch wins over all of them.
