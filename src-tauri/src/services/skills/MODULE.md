# `services::skills`

User skills for the chat: packs of Markdown instructions, one per folder under
`<data dir>/skills/`, each holding a `SKILL.md` in the Agent Skills shape
(YAML frontmatter with `name` + `description`, then the instructions).

## What this module owns

- **Parsing and validation** (`parse_skill_md`) — name rules (lowercase,
  digits, single hyphens, ≤ 64 chars, equal to the folder name), description
  ≤ 1024 chars collapsed to one line, body ≤ `MAX_BODY_CHARS`. Anything else is
  a `SkillLoadError` with a human reason, never a silently dropped skill and
  never a truncated one.
- **The catalog** (`load_catalog`, `catalog_for`, `overview`) — the only I/O.
  Read from disk on each use, so a skill edited in any editor takes effect on
  the next chat turn with no reload step.
- **Planners** — `plan_invocation` / `plan_skill_turn` (does a message start
  with `/name`?), `render_catalog` (the capped `- name: description` lines),
  `render_skill_block` (the `<skill>` block the body travels in),
  `render_planner_rule` (the skills rule for the query planner).
- **The feature gate** — `skills_enabled` (`skills_enabled` preference).
  Experimental and **off by default**.
- **Per-skill switches** — `set_skill_enabled`, stored as a JSON array of
  names in the `skills_disabled` preference. A disabled skill leaves
  `catalog_for` (so the prompt) but stays in `overview`; its folder is never
  touched.
- **Editing from the app** — `read_skill_source`, `save_skill_source` and
  `create_skill` (a template the catalog accepts). A save must load as a
  skill or nothing is written; it is refused if the file changed on disk
  since the editor opened it (`base`); and the file wins on the name — text
  declaring another `name:` renames the folder and moves its switch, unless
  that name is taken. Every name from the frontend goes through
  `check_name` before it is stored or becomes a path; writes go through a
  temp file + rename; toggles are serialized.

## What it does NOT own

- **The chat tool.** `chat::tools::load_skill` advertises the catalog and
  returns a body when the model asks for one.
- **The turn.** `chat::turn::run_chat_turn` calls `plan_skill_turn` and places
  the block; this module never builds a prompt.
- **The UI.** Settings → Skills holds the experimental switch and the folder;
  the Skills view (`src/components/Skills/SkillsView.tsx`) lists skills with
  their switches and edits `SKILL.md`. The files stay the source of truth.
- **Running code.** A skill is instructions only. Scripts or other files in a
  skill folder are ignored — only `.md` / `.txt` files are ever listed, and
  nothing here executes anything.

## The load-bearing decision: progressive disclosure

Modelled on Hermes Agent's skills (and the agentskills.io format), in three
levels:

1. **Index (every turn).** `render_skills_index` — the `- name: description`
   catalog plus the instruction to call `load_skill` FIRST when a skill matches
   and never to claim a load that did not happen — is rendered into the system
   prompt by the tool registry, after the tool list, while `load_skill` is
   available (`ToolRegistry::render_system_prompt_section`). It depends only on
   the folder's contents, sorted by name, so it is byte-identical from turn to
   turn and the llama.cpp KV-prefix anchor keeps matching. It is capped at
   `MAX_CATALOG_CHARS`; skills past the cap are still reachable with `/name`.
2. **Body (on use).** `load_skill(name)` returns the `<skill>` block, which also
   lists the skill's reference files.
3. **Reference files (on demand).** `load_skill(name, file)` returns one of the
   `.md` / `.txt` files under the skill folder (depth ≤ 2, at most
   `MAX_SKILL_FILES`). `read_reference` only serves paths it listed at load
   time, and symlinks are never listed or walked, so nothing outside the skill
   folder is reachable.

A body enters the prompt only on a turn that uses the skill:

- **The planner chooses it**: when the query planner runs, its cached head
  carries `render_planner_rule` and it may add `"skill": "<name>"` next to any
  verdict. `run_chat_turn` places that skill's block exactly like a `/name`
  one — before retrieval, so the sources cannot tempt the model into
  answering without it. The trace records it in `applied_skills`
  (`via: planner` or `slash`).
- **The model chooses it**: it calls `load_skill(name)` and the body comes back
  as a tool result. The fallback when the planner did not run or missed.
- **The user invokes it**: a message starting with `/name` puts the block in
  the **final user message** (never the system message), right before the
  question and after the Sources — placed ahead of the Sources, the model
  followed their citation line instead of the skill's steps. The block says
  it is already loaded, so the index's "load FIRST" does not trigger a second
  `load_skill` round. The rest of the
  message becomes the question that retrieval, the planner and the title see.
  Skills stack — `/a /b request` applies both, in order; parsing stops at the
  first token that is not a skill, so a path in the request survives. A bare
  `/name` asks the skill's description. A `/name` aimed at a switched-off skill
  is sent as plain text with a warning in the output panel. In research mode
  the block rides in the report prompt's tail, and the research estimate is
  keyed on the question without the invocation (`question_of`).
- **History.** Later turns replay the turn with a one-line
  `[skill X was applied to this request]` note instead of the body
  (`chat::turn::history_form`): replaying the body cost context on every turn,
  kept applying an old procedure to unrelated follow-ups and froze a copy the
  user may since have edited. The price is a partial re-prefill on the next
  turn.

With no skills (or the feature off) `load_skill` is hidden by `is_available`,
so an install that never uses skills pays nothing.
