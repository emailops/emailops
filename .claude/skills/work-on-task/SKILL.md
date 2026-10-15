---
name: work-on-task
description: Start or resume work on one roadmap task from tasks/ (T-NNNN) in its own worktree — read the task file, create or reuse the worktree and its feature branch off main, set status doing, keep a Progress line current so the live board (make tasks-serve) shows where it stands, and drive the change with TDD and the repo's gates. Use when the user types /work-on-task T-NNNN, clicks "Open in Herdr" on the task board, or asks to pick up a task by id.
argument-hint: <T-NNNN>
---

# Work on a task

The argument is a task id (`T-0007`). The task file `tasks/<id>-*.md` is the
source of truth for what to build; `tasks/README.md` documents the format.

## 1. Find the task

- If this session already runs in the task's worktree (the board's "Open in
  Herdr" button creates `.claude/worktrees/<id>-<slug>` on
  `feature/<id>-<slug>` and starts Claude there), read `tasks/<id>-*.md` from
  it and skip to step 3. Otherwise read it from the main checkout's `main`
  branch (`git show origin/main:tasks/...` after `git fetch origin main`).
  No such file → say so and stop; do not invent a task.
- If `status` is `done` or `dropped`, say so and ask whether to reopen it.
- If another worktree already has this task at `doing`/`review`
  (`git worktree list`, then read `tasks/<id>-*.md` in each), resume there
  instead of starting a second copy.

## 2. Worktree and branch

- Branch: `feature/<id>-<slug>` (slug = the task file's slug); `fix/` or
  `refactor/` instead when the task is clearly one of those.
- Worktree: `.claude/worktrees/<id>-<slug>` off `origin/main`
  (`git worktree add -b <branch> <path> origin/main`), or reuse the existing
  one. Work only inside it from here on (confirm with
  `git rev-parse --show-toplevel`).

## 3. Mark it started

In the worktree, edit only this task's file:

- `status: doing`
- Append (or update) a `## Progress` section at the end of the body with one
  line: what is done and what is next. Keep it to one line and rewrite it at
  every checkpoint; it is what the board shows.

Commit that change on the branch (`chore: start <id>`). The live board
reads the worktree directly, so it shows the change within seconds.

## 4. Do the work

- Turn the task's `## Acceptance` into tests first. If the section is empty
  or says "to be defined", draft the acceptance criteria, show them to the
  user and wait for an OK before coding; that is a real design fork.
- Follow CLAUDE.md: TDD, the matching skill (`build-ai-feature` for an AI
  surface, `fix-ai-bug` for AI misbehaviour), the `gate-runner` subagent at
  each green checkpoint, and a commit per checkpoint.
- Update the Progress line at each checkpoint.
- Whenever the session produces something the developer will want to open
  (an eval report, a `make verify` summary, a screenshot, a benchmark run),
  record it from the worktree:
  `python3 scripts/tasks.py artifact <path> --label "<what it is>"`. The
  board links the latest one on the card and lists them all in Details. The
  list is gitignored (absolute local paths); the files stay where they are,
  so keep personal-data reports in their gitignored folders.

## 5. Hand off

- When the acceptance tests pass and the gates are green, run the
  `privacy-reviewer` subagent (scope `branch`) and report.
- Push and open the PR only when the user asks. Then set `pr: <number>` and
  `status: review` in the task file, commit, and push that commit.
- After the PR merges the task goes to `done`; `babysit-prs` proposes that
  change.

Never edit other task files from a task branch: parallel branches stay
conflict-free only while each one touches its own task.
