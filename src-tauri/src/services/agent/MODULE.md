# services/agent — the email agent

**Owns:** the user's agent rules and stats panels, the evaluation of new mail and upcoming
calendar events against them, the runs (feed entries) and actions (done / pending / failed
/ rejected) that come out of it, and running an action — at once, or on approval.

**Shape:**
- `planner.rs` — pure: which criteria a trigger is checked against (`plan_criteria`), the
  match and action prompts with a fixed instruction prefix (prefix-cache friendly) and their
  `JsonShape`s (the match reply is a yes/no per criterion, so a small model cannot stop at
  the first one that fits), parsing, validation of the model's actions (`plan_actions`), the approval
  policy (`requires_approval`: mailbox changes always wait), due events, panel windows.
- `runner.rs` — `decide` (the two model calls, no DB — what `make eval-agent` scores on
  synthetic cases in `src-tauri/evals/agent/cases.yaml`), and the executor: `process_new_emails` (sync hook, `agent:emails:*` on
  `ai_background`), `process_due_events` (`agent_event_loop` in `sync_scheduler.rs`,
  `agent:events`), `backfill_panel` (`agent:panel:*`), `run_action` (`agent:action:*` on
  `ai_queue`). `AgentEffects` is the seam for drafts and thread actions.
- `mod.rs` — the surface the commands call: on/off (`agent.enabled`, `agent.since`), rule
  and panel CRUD with validation, feed and actions lists, reject.

**Depends on:** `db/agent.rs` (V035 tables), `emails::generate_draft` / `save_draft` /
`apply_thread_action`, `tasks::create_task`, `skills::catalog_for`, `lenses::extractor`
(body cleaning), `ai::provider::AIProvider`.

**Does not belong here:** sending mail (the agent never sends), chat turns, classification
tags. A new action kind needs a model enum variant, a `CHECK` value in a migration, a line
in `kind_help`, and an arm in `runner::execute`.
