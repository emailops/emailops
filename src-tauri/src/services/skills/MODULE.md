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
  with `/name`?) and `render_skill_block` (the `<skill>` block the body travels
  in).
- **The feature gate** — `skills_enabled` (`skills_enabled` preference).
  **On by default**; a stored `false` keeps it off.
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

- **Bundled skills** (`bundled.rs`) — reviewed skills embedded in the binary
  from `src-tauri/bundled-skills/` (see "Bundled skills" below).

## What it does NOT own

- **The chat tool.** `chat::tools::load_skill` reads a reference file of a
  skill. It never returns a skill's instructions.
- **The turn.** `chat::turn::run_chat_turn` calls `plan_skill_turn` and places
  the block; this module never builds a prompt.
- **The UI.** Settings → Skills holds the switch and the folder;
  the Skills view (`src/components/Skills/SkillsView.tsx`) lists skills with
  their switches and edits `SKILL.md`. The files stay the source of truth.
- **Running code.** A skill is instructions only. Scripts or other files in a
  skill folder are ignored — only `.md` / `.txt` files are ever listed, and
  nothing here executes anything.

## The load-bearing decision: a skill runs only through `/name`

The chat never picks a skill. With skills on and no `/name`, the chat and
planner prompts are byte-identical to an install without skills: there is no
index, no planner rule and no description in the system prefix.

- **The invocation.** A message starting with `/name` puts the block in the
  **final user message** (never the system message), right before the question
  and after the Sources: placed ahead of the Sources, the model followed their
  citation line instead of the skill's steps. The rest of the message becomes
  the question that retrieval, the planner and the title see. Skills stack:
  `/a /b request` applies both, in order; parsing stops at the first token that
  is not a skill, so a path in the request survives. A bare `/name` asks the
  skill's description. A `/name` aimed at a switched-off skill is sent as plain
  text with a warning in the output panel. In research mode the block rides in
  the report prompt's tail, and the research estimate is keyed on the question
  without the invocation (`question_of`).
- **The rest of the turn is unchanged.** Shortcuts, routing, the planner, RAG and
  the tool loop run on the rest of the message. One exception:
  `chat::planner::plan_for_skill_turn` turns a form verdict into a defer, because
  a form stops the turn and would drop the skill the user asked for. A help
  verdict stands: the skill still rides, and the guides answer better.
- **Reference files.** The block lists the `.md` / `.txt` files under the skill
  folder (depth ≤ 2, at most `MAX_SKILL_FILES`), and `load_skill(name, file)` reads
  one. `read_reference` only serves paths it listed at load time, and symlinks
  are never listed or walked, so nothing outside the skill folder is reachable.
  The tool is hidden unless an enabled skill has such a file, and its schema
  depends only on the folder, so the prefix stays stable. Its `enum` then shows
  the names of those skills, and only those.
- **History.** Later turns replay the turn with a one-line
  `[skill X was applied to this request]` note instead of the body
  (`chat::turn::history_form`): replaying the body cost context on every turn,
  kept applying an old procedure to unrelated follow-ups and froze a copy the
  user may since have edited. The price is a partial re-prefill on the next
  turn.

## Bundled skills

`src-tauri/bundled-skills/` holds a copy of `app-skills/` from `emailops-skills`
(`make bundled-skills SKILLS_REPO=…`, which records the commit in `SOURCE`), and
`BUNDLED` embeds each `SKILL.md` with `include_str!`. A test fails if that list
and the folder disagree, if one does not load, or if one holds a reference file:
those are read from disk, so a bundled skill cannot have any.

- **No install step.** A bundled skill is used in place, through its own switch.
  `skills_bundled_on` is an allow list (JSON array of names), because
  `skills_disabled` is a deny list and these are off by default. Nothing is
  copied into `skills/`, at start-up or on update; only `copy_bundled_skill`
  (Edit a copy) writes there.
- **One seam.** `catalog_for` adds the switched-on ones, so `/name` needs no
  bundled-specific code.
- **The user wins.** A folder of the user's with the same name, loadable or not,
  on or off, keeps the bundled skill inactive. `overview` still lists it, as
  `shadowed`.
- **A copy keeps the switch.** Copying a skill that is off leaves the copy off in
  `skills_disabled`, so reading and editing one never puts it in the prompt.
- **It changes with releases.** A user who wants to fix a version uses Edit a
  copy. The release skill calls out any change to the folder in the notes.
