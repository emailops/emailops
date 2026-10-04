# Email agent — specification

Written in ASD-STE100 Simplified Technical English.

Date: 2026-10-03. Branch: `feature/email-agent` (off `feature/competitor-parity`).
Commits: `474314c0` (backend), `d72927b3` (view), `75801412` (eval), and the commit that
adds this specification (reason before each answer, CLI command, Refresh button).

## 1. Purpose

The email agent examines new email and calendar events that start soon. It compares each
item with the rules of the user. When a rule applies, the agent does the actions of that
rule, or it proposes them.

The user sees the work of the agent in the Agent view. The view shows each item as a
message, in the same layout as a chat. A side panel shows the actions. The user approves
or rejects the actions that need approval.

The user can also define stats panels with a prompt. A panel shows the number of emails
that agree with its prompt in a time period.

## 2. Scope

The agent does these tasks:

1. It examines new inbox email after each sync.
2. It examines calendar events that start in the next 15 minutes.
3. It writes reply drafts, creates tasks and runs skills.
4. It proposes to archive, to star and to mark email as read.
5. It counts email for the stats panels.

The agent does not do these tasks:

1. It does not send email. No code path in the agent can send email.
2. It does not examine email that arrived before the user turned the agent on.
3. It does not examine sent email, spam or junk.

## 3. Definitions

| Term | Meaning |
|---|---|
| Rule | A name, a trigger, a classification prompt and an action prompt. |
| Trigger | The type of item that a rule applies to: `email` or `event`. |
| Classification prompt | Text that tells which items the rule applies to. |
| Action prompt | Text that tells what the agent must do with the item. |
| Criterion | One rule or one panel that the model must examine for one item. |
| Run | The result of the agent for one item. One item has one run only. |
| Action | One operation that a run did or proposes. |
| Panel | A counter with a title, a prompt and a period. |

## 4. Functions

### 4.1 Turn the agent on and off

The Agent view has a switch. The agent is off by default.

When the user turns the agent on, the app records the time (`agent.since`). The agent
examines only email with a timestamp at or after this time. Thus, the agent never
examines the history of the mailbox.

The agent also does nothing when the AI master switch is off.

### 4.2 Rules

The user opens the rule dialog with the Rules button. For each rule, the user sets:

1. A name (maximum 80 characters).
2. The trigger: a new email, or an event that starts soon.
3. The account: one account or all accounts.
4. The classification prompt (maximum 2000 characters).
5. The action prompt (maximum 2000 characters).
6. "Ask me before every action". When this is on, all actions of the rule wait for approval.
7. "Enabled". A disabled rule has no effect.

The backend refuses a rule with an empty name or an empty prompt. It also refuses an
account that does not exist.

### 4.3 Email trigger

After each sync batch, the sync queues the task `agent:emails:<account>:<phase>` on the
`ai_background` queue. The task starts after junk scoring. Thus, junk never gets to a rule.

The task finds candidate email with these conditions:

1. The email is in the inbox and is not deleted.
2. The user did not send the email.
3. The timestamp is at or after `agent.since`.
4. The email has no run.
5. The junk detector did not mark the email as junk.

One pass examines a maximum of 25 email. The next sync continues with the rest.

### 4.4 Event trigger

A loop in `sync_scheduler.rs` starts each 60 seconds. If the agent is on and an enabled
event rule exists, the loop queues `agent:events`. The loop never queues a second task
while the first task is in the queue or runs.

The task examines events of calendar-enabled accounts. An event is due when it is not
all-day, not cancelled, and starts in the next 15 minutes. Each event has one run only.

The model gets the event title, start time, location, organizer, attendees and
description. It also gets the 5 latest messages with the attendees.

### 4.5 Decision

The function `runner::decide` makes the decision for one item. It does two steps.

Step 1 (match call). The model gets all applicable criteria and the item. For each
criterion, the model writes a short reason and then "yes" or "no". The JSON shape forces
one answer for each criterion. This prevents a stop after the first criterion that agrees.

Step 2 (action call). For each rule that agrees, the model gets the action prompt, the
permitted actions and the item. The model writes a summary and a maximum of 3 actions.

The planner (`planner::plan_actions`) then removes these actions:

1. An action that the trigger does not permit.
2. An action with an unknown name.
3. A task without a title.
4. A skill that does not exist.
5. A second action of the same type.

A provider error stops the pass before the app records the run. Thus, the agent tries
the item again in the next pass. A reply that the app cannot read gives a failed run.

### 4.6 Actions and approval

| Action | Trigger | Approval | Result |
|---|---|---|---|
| `draft_reply` | email | No | A local reply draft. The app does not push it to the provider. |
| `create_task` | email, event | No | A task with source `agent`. |
| `run_skill` | email, event | No | The text output of the skill, as a note. |
| `mark_read` | email | Always | The thread is marked as read on the provider. |
| `archive` | email | Always | The thread goes to the archive on the provider. |
| `star` | email | Always | The thread gets a star on the provider. |

"Ask me before every action" makes all actions of a rule wait for approval.

The app runs an action that needs no approval immediately after it records the run.

When the user approves an action, the app queues `agent:action:<id>` on `ai_queue`. The
app changes the status from `pending` to `done` before it runs the action. Thus, a second
approval cannot run the action again. If the action fails, the status changes to `failed`
and the app keeps the error text.

When the user rejects an action, the status changes to `rejected`. The action never runs.

### 4.7 Stats panels

The user creates a panel with a title, a prompt and a period: today, last 7 days or last
30 days. "Today" starts at local midnight.

Each panel prompt is one more criterion in the match call of each new email. The app
records each email that agrees. The count is a SQL count of these email in the period.
Thus, the panel costs no model call when the view opens.

When the user creates a panel, the app counts the inbox email that is already in the
period. It examines a maximum of 200 email in the background (`agent:panel:<id>`). When
the user changes the prompt, the app deletes the old results and counts again.

### 4.8 Skills

The action `run_skill` is available only when skills are on and at least one skill is
enabled. The model gets the name and the description of each skill. To run a skill, the
app puts the skill text before the item and gets plain text from the model.

### 4.9 Agent view

The sidebar shows "Agent" when AI is on. The view has these parts:

1. A header with the title, the Refresh button, the Rules button and the switch.
2. A row of panel cards. Each card shows the count, the title and the period.
3. The feed. Each run is a message, with the oldest at the top. A message shows the
   type, the sender, the time, a link to the email, the summary and the actions.
4. The side panel "Actions". It shows the actions to review first, then the recent
   actions. A done reply draft has an "Open draft" link. A skill output opens on a click.

The view loads again when the backend sends the event `agent-updated`.

## 5. Data

Migration `V035__email_agent.sql` adds these tables:

| Table | Content |
|---|---|
| `agent_rules` | The rules. |
| `agent_runs` | One row for each item. `UNIQUE (trigger_kind, trigger_ref)`. |
| `agent_actions` | The actions of each run, with status, result and error. |
| `agent_panels` | The panels. |
| `agent_panel_hits` | The email that agree with each panel. |

All enum columns have a `CHECK` constraint. Deleting a rule keeps its actions with the rule
name. Deleting an email deletes its panel results.

Preferences: `agent.enabled`, `agent.since`, `agent.event_lead_minutes` (default 15,
range 1 to 120).

## 6. Components

| Layer | Files |
|---|---|
| Models | `src-tauri/src/models/agent.rs` |
| Database | `src-tauri/src/db/agent.rs`, `db/calendar.rs` (`get_calendar_event`) |
| Planner (pure) | `src-tauri/src/services/agent/planner.rs` |
| Executor | `src-tauri/src/services/agent/runner.rs` (`AgentEffects` is the test seam) |
| Service surface | `src-tauri/src/services/agent/mod.rs`, `MODULE.md` |
| Hooks | `services/emails/sync.rs`, `services/sync_scheduler.rs`, `services/ai_activity.rs` |
| Commands | `src-tauri/src/commands/agent.rs` (10 commands) |
| CLI | `emailops-cli agent` (feed) and `emailops-cli agent run` |
| Frontend | `src/components/Agent/*`, `src/lib/api.ts`, `src/types/index.ts` |
| Text | `src/locales/{en,es,fr,de}/agent.json` |
| Eval | `src-tauri/src/evals/agent/`, `examples/agent_eval.rs`, `evals/agent/cases.yaml`, `make eval-agent` |

The AI-work dialog (change of provider or model) lists the agent work as `agentRules` and
can stop it.

## 7. Model calls and compatibility

1. All calls are single completions with a JSON shape. The shape is a grammar on
   llama.cpp and a JSON schema on Ollama and OpenRouter.
2. Each prompt has a fixed instruction prefix and a variable suffix. The fixed prefix
   lets the prefix cache of llama.cpp keep the instructions between calls.
3. The agent does not change the chat system prompt. Thus, the chat prefix cache stays
   valid.
4. The model reads a maximum of 3000 characters of the email body.
5. The match reply has a limit of `32 + 50 × criteria` tokens. The action reply has a
   limit of 700 tokens.

## 8. Safety and privacy

1. The agent never sends email.
2. Actions that change the mailbox on the provider always wait for approval.
3. Reply drafts stay local until the user opens them.
4. The agent examines only email that arrives after the user turned it on.
5. The eval uses only synthetic cases on an in-memory database. It reads no mailbox.

The decision is in `docs/DECISIONS.md`, entry "The email agent acts alone only on local,
reversible actions" (2026-10-03).

## 9. Verification

These results come from runs on 2026-10-03 and 2026-10-04.

| Check | Command | Result |
|---|---|---|
| All gates | `make gates SET=all` (2026-10-04, final code) | rust-test 4122 passed, 0 failed; clippy, clippy-desktop, fmt, biome, tsc OK; vitest 2774 passed. `outdated` failed: 5 dependencies have new upstream releases (not from this change). |
| Agent unit and integration tests | `cargo test --no-default-features --features eval --lib -- agent` | 59 passed, 0 failed |
| CLI tests | `cargo test --no-default-features --features cli --lib -- cli::` | 155 passed, 0 failed |
| Agent view tests | `npx vitest run src/components/Agent` (with the Refresh button) | 9 passed |
| Eval | `make eval-agent`, model `qwen3.5-4b-q4_k_m` on llamacpp | 9/9 cases, two runs |

The app was also run on the synthetic demo instance (`verify-emailops`). These steps
passed:

1. Rules and a panel created through the rule dialog and the panel dialog.
2. `emailops-cli agent run` on 3 new synthetic email. The support email got a local draft
   and a task. The promotion got two pending actions. The third email had no match.
3. The panel "Support today" counted 1 email.
4. Approve on `archive` ran the action. It failed with "Authentication required", because
   the demo accounts have no credentials. The app recorded the error.
5. Reject on `mark_read` changed the status to `rejected`.
6. "Open draft" opened the reply composer with the draft of the agent.
7. The event loop of the running app examined a synthetic event 10 minutes before it
   started, without the CLI. The agent created a preparation task.

## 10. Limits

1. The agent does not examine email in the demo instance, because demo accounts do not
   sync. Use `emailops-cli agent run` to start a pass by hand.
2. An event is examined only while the app runs, in the 15 minutes before it starts.
3. A panel counts back a maximum of 200 email when the user creates it.
4. The match call takes approximately 0.5 to 2.5 seconds for each email on the 4B model.
   The agent runs on the background queue, after classification.
5. When the window of the app is hidden, CSS transitions stop. In automated screenshots,
   the switch can show the old position. This is not a defect of the agent.

## 11. Open items

1. Update the user docs (`docs/site`) and `docs/cli.md` for the Agent view and the CLI
   command (`maintain-docs` skill).
2. Add the agent to the verification map (`maintain-verification` skill).
3. Record the demo video.
