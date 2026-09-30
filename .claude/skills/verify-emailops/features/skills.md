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

## How to get to it (user POV)

- Settings (gear, `aria/Application settings`) → **AI Skills** → **Enable skills** on.
- Sidebar → AI Features → **Skills**.
- In the chat, start a message with `/`.

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
