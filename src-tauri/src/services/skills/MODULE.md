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
  `render_skill_block` (the `<skill>` block the body travels in).
- **The feature gate** — `skills_enabled` (`skills_enabled` preference,
  default on).

## What it does NOT own

- **The chat tool.** `chat::tools::load_skill` advertises the catalog and
  returns a body when the model asks for one.
- **The turn.** `chat::turn::run_chat_turn` calls `plan_skill_turn` and places
  the block; this module never builds a prompt.
- **Editing skills.** They are files; Settings only lists them and opens the
  folder (`commands::skills`).
- **Running code.** A skill is instructions only. Scripts or other files in a
  skill folder are ignored — only `.md` / `.txt` files are ever listed, and
  nothing here executes anything.

## The load-bearing decision: progressive disclosure

Modelled on Hermes Agent's skills (and the agentskills.io format), in three
levels:

1. **Index (every turn).** `render_skills_index` — the `- name: description`
   catalog plus the instruction to call `load_skill` FIRST when a skill matches
   and never to claim a load that did not happen — is rendered into the system
   prompt through the `load_skill` tool's `prompt_appendix`. It depends only on
   the folder's contents, sorted by name, so it is byte-identical from turn to
   turn and the llama.cpp KV-prefix anchor keeps matching. It is capped at
   `MAX_CATALOG_CHARS`; skills past the cap are still reachable with `/name`.
2. **Body (on use).** `load_skill(name)` returns the `<skill>` block, which also
   lists the skill's reference files.
3. **Reference files (on demand).** `load_skill(name, file)` returns one of the
   `.md` / `.txt` files under the skill folder (depth ≤ 2, at most
   `MAX_SKILL_FILES`). `read_reference` only serves paths it listed at load
   time, so nothing outside the skill folder is reachable.

A body enters the prompt only on a turn that uses the skill:

- **The model chooses it**: it calls `load_skill(name)` and the body comes back
  as a tool result.
- **The user invokes it**: a message starting with `/name` puts the block in
  the **final user message** (never the system message), and the rest of the
  message becomes the question that retrieval, the planner and the title see.
  Skills stack — `/a /b request` applies both, in order; parsing stops at the
  first token that is not a skill, so a path in the request survives. The block
  is persisted with the rest of the prompted tail, so later turns in the
  conversation replay it byte-identically.

With no skills (or the feature off) `load_skill` is hidden by `is_available`,
so an install that never uses skills pays nothing.
