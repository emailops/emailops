# Chat skills

Saved procedures the chat follows for one kind of request. Each skill is a folder in the
data dir, `skills/<name>/SKILL.md` (frontmatter `name` + `description`, then instructions;
optional `.md`/`.txt` reference files). Experimental and **off by default**.

## Sub-features

- `skills.toggle` Settings → **AI Skills** holds **Enable skills** (Experimental badge). Off: no sidebar entry, `load_skill` hidden, planner prompt unchanged.
- `skills.view` Sidebar **Skills** (`data-testid="sidebar-skills"`) opens the two-pane view: list with a per-skill switch (`skill-toggle-<name>`) on the left, `SKILL.md` editor (`skill-editor`) on the right.
- `skills.new` **New** (`skill-new`, `skill-new-name`, `skill-create`) writes a template the catalog accepts.
- `skills.save` **Save** (`skill-save`) validates with the catalog's parser, refuses a file changed on disk since opening (`skill-reload` offers "Reload from disk"), and renames the skill when the text declares another `name:`.
- `skills.delete` **Delete** (`skill-delete` → `skill-delete-confirm` / `skill-delete-cancel`) moves the folder to `skills/.deleted`.
- `skills.slash` typing `/` in the chat input lists enabled skills (`slash-option-<name>`); Enter/Tab fills `/name `, Escape dismisses.
- `skills.apply` a turn applies a skill via the query planner (`"skill"` next to its verdict), `/name`, or the model's `load_skill`; the reasoning trace shows a **Skill** step.
- `skills.list` every entry lists the skills found in the folder as `/name` with its description.
- `skills.switchOne` a per-skill switch turns one skill off: it leaves the `/` list and the planner's skill index but stays listed.
- `skills.loadErrors` a `SKILL.md` that fails to parse is listed with its translated reason instead of silently missing.
- `skills.folder` the skills folder is shown; **Open folder** creates it if needed and opens it; **Reload** re-reads it.

## How to get to it (user POV)

Four entry points, the columns of `## Parity`:

- **Chat input /** — start a message with `/` in the docked panel or the full chat view (the same ChatInput).
- **Skills view** — Sidebar → AI Features → **Skills** (only while skills are on).
- **Settings → AI Skills** — gear (`aria/Application settings`) → **AI Skills**: **Enable skills**, the folder path, Open folder / Reload, a read-only list.
- **emailops-cli skills** — `make cli-demo ARGS="skills --json"`, a read-only list plus load errors; skills are applied from `emailops-cli chat "/name …"` through the same backend.

## Parity

| Capability | Chat input / | Skills view | Settings → AI Skills | emailops-cli skills |
|---|---|---|---|---|
| skills.toggle | vitest:src/components/Chat/ChatInput.slash.test.tsx::stays out of the way while skills are off | n/a: the view only exists while skills are on | e2e:Skills/activar | gap: missing — the CLI only reports that skills are off; no command switches them |
| skills.view | n/a: an invocation surface, not a skill browser; its list is skills.slash | e2e:Skills/vista | n/a: Settings is read-only by design; the two-pane editor belongs to the view | n/a: no two-pane editor in a terminal; the listing is skills.list |
| skills.list | n/a: the / menu lists enabled skills; that is skills.slash | vitest:src/components/Skills/SkillsView.test.tsx::lists every skill and opens the first one in the editor | vitest:src/components/Settings/SkillsSettings.test.tsx::lists each skill as its slash command with its description | gap: untested — the only dispatch test runs with no data dir and asserts Ok; the renderer is untested |
| skills.switchOne | vitest:src/components/Chat/ChatInput.slash.test.tsx::offers the enabled skills that match what is typed | vitest:src/components/Skills/SkillsView.test.tsx::switches a skill off from its toggle | gap: missing — lists every skill as /name with no on/off state or switch | gap: missing — pretty mode ignores enabled and no command switches a skill |
| skills.new | n/a: an invocation surface, not an editor | vitest:src/components/Skills/SkillsView.test.tsx::creates a new skill and opens it | n/a: by design Settings only opens the folder; creating belongs to the view | gap: missing — skills is list-only; no create subcommand |
| skills.save | n/a: an invocation surface, not an editor | vitest:src/components/Skills/SkillsView.test.tsx::loads the selected skill into the editor and saves the edit | n/a: by design Settings does not edit skills | gap: missing — no save or validate subcommand |
| skills.delete | n/a: an invocation surface, not an editor | vitest:src/components/Skills/SkillsView.test.tsx::deletes a skill only after the user confirms | n/a: by design Settings does not edit skills | gap: missing — no delete subcommand |
| skills.slash | vitest:src/components/Chat/ChatInput.slash.test.tsx::Enter picks the highlighted skill instead of sending | n/a: the view has no chat input | n/a: Settings has no chat input | n/a: no input completion; in the REPL / already means a REPL command |
| skills.apply | rust:src-tauri/src/services/chat/planner.rs::plan_search_returns_the_skill_the_planner_named | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point |
| skills.loadErrors | n/a: a broken skill is never offered | vitest:src/components/Skills/SkillsView.test.tsx::shows why a skill in the folder failed to load, translated | vitest:src/components/Settings/SkillsSettings.test.tsx::shows why a skill failed to load | gap: untested — the not-loaded lines have no test |
| skills.folder | n/a: the chat input does not manage files | gap: missing — the view shows no folder path and cannot open it | vitest:src/components/Settings/SkillsSettings.test.tsx::opens the skills folder and re-reads it on demand | gap: untested — prints the folder, untested |

## Driving it with verify.sh

Preconditions: demo instance (`$V launch`); the demo DB has no skills and the feature off unless a previous run left them (check `ls .emailops-demo-data/skills` and `sqlite3 .emailops-demo-data/emailops.db "select key,value from user_preferences where key like 'skills%'"`). Handles confirmed live on 29/09/2026.

- Turn on → `$V wd click 'aria/Application settings'`, click the last button containing `AI Skills`, then the `[role=switch]` in the row labelled `Enable skills`; close → `$V wd exists '[data-testid="sidebar-skills"]'` is present and `skills_enabled|true` is in `user_preferences`.
- Create → `$V wd click '[data-testid="sidebar-skills"]'`, `$V wd click '[data-testid="skill-new"]'`, `$V wd type '[data-testid="skill-new-name"]' 'weekly-digest'`, `$V wd click '[data-testid="skill-create"]'` → `.emailops-demo-data/skills/weekly-digest/SKILL.md` exists and the editor shows the template.
- Invalid save → type text without the `---` block, `$V wd click '[data-testid="skill-save"]'` → `$V wd text '[data-testid="skills-error"]'` names the missing frontmatter and the file hash is unchanged.
- Switch off → `$V wd click '[data-testid="skill-toggle-weekly-digest"]'` → `skills_disabled|["weekly-digest"]`.
- Clean up afterwards: delete the skill from the view (or its folder) and `delete from user_preferences where key in ('skills_enabled','skills_disabled')` — the chat eval installs its own skills named `vendor-support`, `travel-brief`, `weekly-digest`… and refuses to overwrite existing folders.

## Gotchas

- Any source edit while a `tauri dev` instance runs from the same worktree rebuilds and restarts the app: never edit Rust during `make docs-check ARGS=--with-app` or `make verify`.
- The webdriver build rewrites `src-tauri/gen/schemas/*.json` (wdio-webdriver permissions); `git checkout -- src-tauri/gen/schemas` before committing.
- `/name` of a switched-off skill is sent as plain text with a warning in the output panel, by design.

| Case | Test kind |
|---|---|
| parsing, naming, catalog, planner rule, block wording, delete/rename/conflict | unit (`services::skills::tests`) |
| index placement, `load_skill` schema, trace step, history note | unit (`services::chat::*`) |
| full lifecycle on disk | integration (`skill_lifecycle_through_the_service`) |
| view, toggle, editor races, `/` suggestions | vitest (`SkillsView`, `ChatInput.slash`, `slashSkills`) |
| skill command arguments | contract (`src/lib/apiContract/skills.api.test.ts`) |
| sidebar entry and view | e2e (`Skills/activar`, `Skills/vista` in `sweep.mjs`) |
| selection, `/name`, negatives on the demo mailbox | eval (`skill_*` in `src-tauri/evals/chat/cases/skills.yaml`) |
