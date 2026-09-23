# `services::forms`

Fillable app forms: the declarative field definitions the chat hands to the
model, and the parser that maps a model reply back onto them.

## What this module owns

- **`registry.rs`** — one `FormDef` per fillable form, each a flat list of typed
  `FieldDef`s whose `description` is written *for the model*, not for the UI
  (the UI keeps its own i18n labels). Plus `lookup()` and `catalog()`.
- **`filler.rs`** — the pure `parse_fill` (model text → `FormFill`) and the thin
  `fill_form` executor that renders `forms.fill`, calls the provider and parses.

## What it does NOT own

- **Applying the values.** A fill is a *proposal*: the frontend opens the form
  with the values in it and the user submits. Nothing here writes to the DB, and
  no form is ever saved without the user pressing the form's own button.
- **The turn.** `services::chat::form_turn` runs the turn, emits the effect and
  composes the reply. This module is only definitions + parsing.
- **The UI.** The frontend renders its own components; `FormDef` is the contract
  they agree on, not a renderer.

## The two load-bearing decisions

**1. A form's field definitions never enter the chat system prompt.**

The filler runs as its own focused one-shot completion (the same shape as
`chat::planner::plan_search`), on the scratch sequence with `cache_prompt =
false`. So the ~700 tokens of the Create Lens field definitions cost exactly
nothing on an ordinary chat turn, and never touch the chat KV prefix. The only
forms text that rides in a prompt on every turn is `catalog()` — one
`id: summary` line per form, asserted under 600 chars by a unit test, living in
the *query planner's* cached head.

**2. The planner decides whether a turn is a form turn; an open form only
decides which one.**

`chat::view_context::resolve_target_form` enforces it. Letting a form on screen
turn every question into a fill would hijack "qué correos tengo hoy" asked with
the Create Lens dialog up — the same trap `docs/DECISIONS.md` (2026-09-14)
records for routing: context is a hint, never a gate.

## Adding a form

1. Add a `FormDef` to `registry::FORMS`. Keep `summary` to one short line — it
   is paid for on every planner call. `target` must parse as a
   `help_docs::nav::NavTarget` (a unit test enforces this).
2. Add its id to `FILLABLE_FORM_IDS` in `src/lib/chatToolEffects.ts`.
3. Have the component that renders the form call `useChatFilledForm('<id>')`
   (`src/stores/formFillStore.ts`) and register itself in `viewContextStore`
   while open, so an edit request targets it.
4. Add cases to `src-tauri/evals/forms/cases.yaml` and run `make eval-forms`.

Steps 1 and 4 are the work; 2 and 3 are one line each. Nothing in `filler.rs`
changes — the parser is driven entirely by the `FormDef`.

## Depends on

- `services::prompts` for the `forms.fill` template (user-editable).
- `ai::provider::AIProvider` for the one completion.
- `models::lens` — the registry's Create Lens enums mirror it, and unit tests
  fail if the two drift.
