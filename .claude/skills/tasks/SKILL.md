---
name: tasks
description: Manage the roadmap tasks from a Claude session instead of the task board page — list what is pending (by status, milestone, or whose agent needs you), show one task, start or focus a task's session (a Herdr tab, a background session or a cloud session), move a task to another milestone, add a milestone, delete a card or discard a worktree. Same code and same rules as the live board (make tasks-serve). Use when the user types /tasks, asks "qué tareas hay / tareas pendientes / qué está en curso / quién me necesita", "empieza / abre / arranca la T-NNNN" (also "en segundo plano" or "en la nube"), "mueve la T-NNNN a <milestone>", "borra la T-NNNN", or wants the board's state without opening it. To create a task use the new-task skill; to do the work inside a task's session use work-on-task.
argument-hint: "[list|show|start|focus|milestone|delete|discard] [T-NNNN] [...]"
---

# Tasks from a session

Every action goes through `scripts/task_board.py`, the same code the board's
buttons call, so the session and the page never disagree. `tasks/README.md`
documents the task format and how the board merges its sources. Run the
commands from the repo root of any checkout of this repo (the main checkout or
a worktree): the script finds the main checkout itself. If `scripts/task_board.py`
has no `list` subcommand, the checkout predates this skill: say so.

## Read

The board reads `origin/main`, so bring it up to date first:
`git fetch -q origin` (once per turn is enough).

| The user wants | Run |
| --- | --- |
| what is open (todo, doing, review) | `python3 scripts/task_board.py list` |
| everything, done and dropped included | `... list --all` |
| one status or milestone | `... list --status doing --status review`, `... list --milestone 0.7.0` |
| which agents wait for them | `... list --needs-you` |
| a one-line count | `... summary` |
| one task: details, branch, agent, artifacts | `... show T-NNNN` |

Add `--json` when you need to compute on the result (filter on title words,
count, compare). For the user, show the plain table, or a short summary in
their language if they asked a question ("2 en curso, T-0004 te espera").
Don't paste a task's whole body unless they ask for it.

## Start or focus a task

`python3 scripts/task_board.py open T-NNNN [--mode herdr|bg|cloud]`

- **herdr** (the default, the board's **Open in Herdr**): if an agent already
  runs on the task it is focused. Otherwise it creates or reuses the worktree
  `.claude/worktrees/T-NNNN-slug` on `feature/T-NNNN-slug`, opens a Herdr tab
  there, starts Claude and types `/work-on-task T-NNNN` without sending it.
- **bg** ("en segundo plano", "sin abrir ventana"): same worktree, then a
  background Claude session named after the task that runs
  `/work-on-task T-NNNN`. The user follows it with `claude attach T-NNNN`;
  `claude agents` lists the background sessions. The board shows it like any
  agent in the worktree.
- **cloud** ("en la nube", "remoto"): a Claude cloud session that runs
  `/work-on-task T-NNNN`, started in a Herdr tab at the main checkout so it
  stays attached. It has no local mailbox, models or private evals: say so if
  the task needs `make cli-run`, evals over private data or the demo app. The
  board sees it only once it pushes its `feature/T-NNNN-*` branch.

Before starting, check the task with `show`: if it is `done` or `dropped`, say
so and ask. If its agent is waiting (`needs you`), `focus` is what the user
wants. Start only the task(s) the user named; one `open` per task.

Do not do the task's work in this session: the started session owns it, through
`/work-on-task`.

## Change

| The user wants | Run |
| --- | --- |
| focus the task's agent and raise its terminal | `... focus T-NNNN` |
| move a task to a milestone | `... milestone T-NNNN <name>` |
| add a milestone | `... new-milestone <name>` |
| delete a card (worktree, branch and agent stay) | `... delete T-NNNN` |
| remove the worktree, local branch and Herdr panes | `... discard T-NNNN`, then `--yes` |

- The milestone must exist (`tasks/milestones.md`); if it does not, ask whether
  to add it with `new-milestone` first.
- `delete` and `discard` are what the user asked for by id, never a cleanup you
  pick. `discard` first runs without `--yes`: it prints what would be lost
  (changed files, commits on no other branch). Show that line and ask before
  re-running with `--yes`. A pushed branch and its PR stay on GitHub.
- Edits are local commits on `chore/tasks-board`, or on the task's own branch
  once it started. Nothing is pushed.

A non-zero exit prints `error: <reason>`: report it, don't work around it.

## Report

One line per action, in the user's language: the task id, what happened, and
where to follow it (the Herdr tab, `claude attach T-NNNN`, the cloud session).
