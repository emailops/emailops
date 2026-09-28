//! User skills: packs of instructions the chat loads only when they are needed.
//!
//! A skill is a folder under `<data dir>/skills/` holding a `SKILL.md` file —
//! the same shape as Anthropic's Agent Skills:
//!
//! ```text
//! ---
//! name: weekly-summary
//! description: Summarise the week's mail by client. Use when the user asks for a weekly recap.
//! ---
//! Group the mail by client, one heading each, newest first…
//! ```
//!
//! The load-bearing decision is *progressive disclosure* (see `MODULE.md`):
//! only the one-line catalog (`name: description`) rides in the chat prompt,
//! inside the `load_skill` tool's schema; the body enters the prompt only on a
//! turn that uses the skill — when the model calls `load_skill`, or when the
//! user types `/name` at the start of a message.
//!
//! Split as pure planners (`parse_skill_md`, `plan_invocation`,
//! `render_catalog`, `render_skill_block`) plus one thin I/O executor
//! (`load_catalog`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::db::Database;

/// File every skill folder must contain.
pub const SKILL_FILE: &str = "SKILL.md";

/// Folder under the data dir that holds one sub-folder per skill.
pub const SKILLS_DIR: &str = "skills";

/// Preference key gating the whole feature (default on: with no skills on
/// disk the feature costs nothing).
pub const SKILLS_ENABLED_PREF: &str = "skills_enabled";

/// Longest accepted skill name (Agent Skills spec).
pub const MAX_NAME_CHARS: usize = 64;

/// Longest accepted description (Agent Skills spec).
pub const MAX_DESCRIPTION_CHARS: usize = 1024;

/// Longest accepted body. A skill body rides in the final user message of
/// every turn that uses it, and the smallest supported machine runs an
/// 8192-token window shared with the system prompt, the tools, the sources and
/// the answer. ~8000 chars is ~2000 tokens: room for real instructions without
/// front-truncating the prompt. A longer skill is rejected, never cut, so the
/// model never follows half of a procedure.
pub const MAX_BODY_CHARS: usize = 8000;

/// Budget for the whole catalog advertised in the `load_skill` schema. It sits
/// in the system prompt on every turn, so it is capped; skills past the cap
/// stay usable via `/name`.
pub const MAX_CATALOG_CHARS: usize = 2000;

/// Most supporting files listed per skill, and how deep under the skill
/// folder they are looked for (`references/escalation.md` is depth 2).
pub const MAX_SKILL_FILES: usize = 20;
const MAX_FILE_DEPTH: usize = 2;

/// Supporting files are instructions too: only plain text is listed. Scripts
/// and binaries are ignored — nothing in a skill folder is ever executed.
const TEXT_EXTENSIONS: &[&str] = &["md", "txt"];

/// One skill parsed from disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// Markdown instructions after the frontmatter, trimmed.
    pub body: String,
    /// Path of the `SKILL.md` it came from (empty when parsed from a string).
    pub path: PathBuf,
    /// Supporting text files in the skill folder (`references/…`, templates),
    /// relative to it — what `load_skill(name, file)` may read.
    pub files: Vec<String>,
}

/// A `SKILL.md` that could not be loaded, and why — surfaced in Settings and
/// the CLI so a typo does not make a skill vanish silently.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillLoadError {
    pub path: PathBuf,
    pub message: String,
}

/// Everything found in the skills folder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SkillCatalog {
    /// Valid skills, sorted by name (stable order keeps the prompt prefix,
    /// and so the KV cache, identical from turn to turn).
    pub skills: Vec<Skill>,
    pub errors: Vec<SkillLoadError>,
}

impl SkillCatalog {
    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.iter().find(|s| s.name == name)
    }

    pub fn names(&self) -> Vec<&str> {
        self.skills.iter().map(|s| s.name.as_str()).collect()
    }
}

#[derive(Debug, Deserialize)]
struct Frontmatter {
    name: Option<String>,
    description: Option<String>,
}

/// Parse the text of a `SKILL.md`. `folder` is the name of the folder that
/// holds it: the frontmatter `name` must match it, as in the Agent Skills
/// spec, so the name the user types (`/name`) is the one they see on disk.
pub fn parse_skill_md(text: &str, folder: &str) -> Result<Skill, String> {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let rest = text
        .strip_prefix("---\n")
        .ok_or("SKILL.md must start with a `---` frontmatter block holding `name` and `description`")?;
    let (yaml, body) = match rest.find("\n---") {
        Some(end) => {
            let after = &rest[end + 4..];
            // The closing fence must be a line of its own.
            let after = match after.find('\n') {
                Some(nl) if after[..nl].trim().is_empty() => &after[nl + 1..],
                None if after.trim().is_empty() => "",
                _ => return Err("the frontmatter's closing `---` must be on its own line".to_string()),
            };
            (&rest[..end], after)
        }
        None => return Err("the frontmatter block is never closed with `---`".to_string()),
    };
    let fm: Frontmatter = serde_yaml::from_str(yaml).map_err(|e| format!("invalid frontmatter: {e}"))?;

    let name = fm.name.map(|n| n.trim().to_string()).unwrap_or_default();
    if name.is_empty() {
        return Err("the frontmatter has no `name`".to_string());
    }
    validate_name(&name)?;
    if name != folder {
        return Err(format!(
            "`name: {name}` must match its folder name `{folder}` (rename one of them)"
        ));
    }

    let description = fm
        .description
        .map(|d| d.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    if description.is_empty() {
        return Err("the frontmatter has no `description` (say what the skill does and when to use it)".to_string());
    }
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(format!(
            "the description is longer than {MAX_DESCRIPTION_CHARS} characters"
        ));
    }

    let body = body.trim().to_string();
    if body.is_empty() {
        return Err("the skill has no instructions after the frontmatter".to_string());
    }
    if body.chars().count() > MAX_BODY_CHARS {
        return Err(format!(
            "the instructions are longer than {MAX_BODY_CHARS} characters; shorten them so they fit the model's context"
        ));
    }

    Ok(Skill {
        name,
        description,
        body,
        path: PathBuf::new(),
        files: Vec::new(),
    })
}

/// Agent Skills naming: lowercase ASCII letters, digits and single hyphens,
/// not starting or ending with a hyphen, at most `MAX_NAME_CHARS`.
fn validate_name(name: &str) -> Result<(), String> {
    let valid = name.len() <= MAX_NAME_CHARS
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if valid {
        Ok(())
    } else {
        Err(format!(
            "`{name}` is not a valid skill name: use lowercase letters, digits and single hyphens, at most {MAX_NAME_CHARS} characters"
        ))
    }
}

/// A message that starts with `/name` for a known skill: the skill to load and
/// what the user asked besides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    /// Every skill named, in the order typed, without repeats.
    pub names: Vec<String>,
    /// The message without the `/name` prefix, trimmed. May be empty.
    pub rest: String,
}

/// Decide whether `message` invokes a skill. Only a leading `/name` naming a
/// known skill counts, so a path or a date ("/2026") typed mid-sentence, or a
/// slash command for something else, is left alone.
pub fn plan_invocation(message: &str, names: &[&str]) -> Option<Invocation> {
    let mut rest = message.trim_start();
    let mut found: Vec<String> = Vec::new();
    // Stack `/a /b …` until the first token that is not a known skill, so a
    // path or a date in the request itself is left alone.
    while let Some(after_slash) = rest.strip_prefix('/') {
        let end = after_slash.find(char::is_whitespace).unwrap_or(after_slash.len());
        let typed = after_slash[..end].to_ascii_lowercase();
        let Some(name) = names.iter().find(|n| **n == typed) else {
            break;
        };
        if !found.iter().any(|f| f == name) {
            found.push((*name).to_string());
        }
        rest = after_slash[end..].trim_start();
    }
    if found.is_empty() {
        return None;
    }
    Some(Invocation {
        names: found,
        rest: rest.trim().to_string(),
    })
}

/// A chat turn the user started with `/name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillTurn {
    pub skills: Vec<String>,
    /// What the turn asks: the text after `/name`, or a request to apply the
    /// skill when the user typed nothing else.
    pub question: String,
    /// `render_skill_block` of the skill, for the final user message.
    pub block: String,
}

/// Plan a turn whose message may start with `/name`. `None` for any message
/// that does not invoke a skill in `catalog`.
pub fn plan_skill_turn(message: &str, catalog: &SkillCatalog) -> Option<SkillTurn> {
    let invocation = plan_invocation(message, &catalog.names())?;
    let skills: Vec<&Skill> = invocation.names.iter().filter_map(|n| catalog.get(n)).collect();
    if skills.is_empty() {
        return None;
    }
    let quoted: Vec<String> = skills.iter().map(|s| format!("\"{}\"", s.name)).collect();
    let question = match (invocation.rest.is_empty(), quoted.len()) {
        (false, _) => invocation.rest,
        (true, 1) => format!("Apply the skill {}.", quoted.join("")),
        (true, _) => format!("Apply the skills {}.", quoted.join(", ")),
    };
    Some(SkillTurn {
        skills: skills.iter().map(|s| s.name.clone()).collect(),
        question,
        block: skills
            .iter()
            .map(|s| render_skill_block(s))
            .collect::<Vec<_>>()
            .join("\n\n"),
    })
}

/// The catalog lines advertised to the model: `- name: description`, sorted,
/// capped at `MAX_CATALOG_CHARS`. Returns the text and the names it lists.
pub fn render_catalog(skills: &[Skill]) -> (String, Vec<String>) {
    let mut sorted: Vec<&Skill> = skills.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    let mut lines: Vec<String> = Vec::new();
    let mut names = Vec::new();
    let mut used = 0;
    for s in sorted {
        let line = format!("- {}: {}", s.name, s.description);
        // +1 for the newline that joins it to the previous line.
        let cost = line.chars().count() + usize::from(!lines.is_empty());
        if used + cost > MAX_CATALOG_CHARS {
            break;
        }
        used += cost;
        lines.push(line);
        names.push(s.name.clone());
    }
    (lines.join("\n"), names)
}

/// The system-prompt section listing the skills, Hermes-style: the catalog
/// plus the instruction to load a matching skill before answering. `None`
/// with no skills.
pub fn render_skills_index(skills: &[Skill]) -> Option<String> {
    let (lines, names) = render_catalog(skills);
    if names.is_empty() {
        return None;
    }
    Some(format!(
        "Skills (the user's saved procedures):\n{lines}\n\
Before replying, scan the skills above. If one matches the request, call load_skill with its name FIRST and follow the instructions it returns. Never say you loaded a skill without calling load_skill."
    ))
}

/// Read one of `skill`'s supporting files. Only a path listed in
/// `skill.files` is served, so nothing outside the skill folder is reachable.
pub fn read_reference(skill: &Skill, file: &str) -> Result<String, String> {
    let wanted = file.trim().trim_start_matches("./");
    let Some(listed) = skill.files.iter().find(|f| f.as_str() == wanted) else {
        return Err(if skill.files.is_empty() {
            format!("The skill \"{}\" has no reference files.", skill.name)
        } else {
            format!(
                "The skill \"{}\" has no file \"{wanted}\". Its files: {}.",
                skill.name,
                skill.files.join(", ")
            )
        });
    };
    let dir = skill
        .path
        .parent()
        .ok_or_else(|| format!("The skill \"{}\" has no folder.", skill.name))?;
    let text = std::fs::read_to_string(dir.join(listed)).map_err(|e| format!("Could not read {listed}: {e}"))?;
    if text.chars().count() <= MAX_BODY_CHARS {
        return Ok(text);
    }
    let cut: String = text.chars().take(MAX_BODY_CHARS).collect();
    Ok(format!("{cut}\n[truncated]"))
}

/// The supporting text files under a skill folder, relative and sorted.
fn list_skill_files(dir: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if depth < MAX_FILE_DEPTH {
                    walk(root, &path, depth + 1, out);
                }
                continue;
            }
            let is_text = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| TEXT_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()));
            let Ok(rel) = path.strip_prefix(root) else {
                continue;
            };
            let rel = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            if is_text && rel != SKILL_FILE {
                out.push(rel);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, 1, &mut out);
    out.sort();
    out.truncate(MAX_SKILL_FILES);
    out
}

/// The block that carries a skill's instructions into a prompt.
pub fn render_skill_block(skill: &Skill) -> String {
    let files = if skill.files.is_empty() {
        String::new()
    } else {
        format!(
            "\n\nReference files — read one with load_skill(name=\"{}\", file=…) when the instructions need it: {}",
            skill.name,
            skill.files.join(", ")
        )
    };
    format!(
        "<skill name=\"{}\">\nThe user's skill \"{}\" applies to this request. Follow its instructions:\n\n{}{files}\n</skill>",
        skill.name, skill.name, skill.body
    )
}

/// `<data dir>/skills`, derived from where the database lives. `None` for an
/// in-memory test database, which has no data dir.
pub fn skills_dir(db: &Database) -> Option<PathBuf> {
    let parent = db.db_path().parent()?;
    if parent.as_os_str().is_empty() {
        return None;
    }
    Some(parent.join(SKILLS_DIR))
}

/// Whether the feature is on (default on).
pub fn skills_enabled(db: &Database) -> bool {
    db.get_preference(SKILLS_ENABLED_PREF)
        .ok()
        .flatten()
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(true)
}

/// Read every `<dir>/<folder>/SKILL.md`. A missing folder is an empty catalog,
/// not an error: most installs have no skills.
pub fn load_catalog(dir: &Path) -> SkillCatalog {
    let mut catalog = SkillCatalog::default();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return catalog,
        Err(e) => {
            catalog.errors.push(SkillLoadError {
                path: dir.to_path_buf(),
                message: format!("cannot read the skills folder: {e}"),
            });
            return catalog;
        }
    };
    for entry in entries.flatten() {
        let folder_path = entry.path();
        if !folder_path.is_dir() {
            continue;
        }
        let file = folder_path.join(SKILL_FILE);
        if !file.is_file() {
            continue;
        }
        let folder = entry.file_name().to_string_lossy().into_owned();
        let parsed = std::fs::read_to_string(&file)
            .map_err(|e| format!("cannot read the file: {e}"))
            .and_then(|text| parse_skill_md(&text, &folder));
        match parsed {
            Ok(mut skill) => {
                skill.files = list_skill_files(&folder_path);
                skill.path = file;
                catalog.skills.push(skill);
            }
            Err(message) => catalog.errors.push(SkillLoadError { path: file, message }),
        }
    }
    catalog.skills.sort_by(|a, b| a.name.cmp(&b.name));
    catalog.errors.sort_by(|a, b| a.path.cmp(&b.path));
    catalog
}

/// The catalog for this install, or empty when the feature is off or there is
/// no data dir.
pub fn catalog_for(db: &Database) -> SkillCatalog {
    if !skills_enabled(db) {
        return SkillCatalog::default();
    }
    skills_dir(db).map(|d| load_catalog(&d)).unwrap_or_default()
}

/// One skill as listed in Settings and the CLI — no body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
}

/// What Settings and `emailops-cli skills` show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsOverview {
    pub enabled: bool,
    /// Where skills live; `None` only without a data dir (tests).
    pub dir: Option<PathBuf>,
    pub skills: Vec<SkillInfo>,
    pub errors: Vec<SkillLoadError>,
}

/// List the skills folder whether or not the feature is on, so the user can
/// see what turning it on would give the chat.
pub fn overview(db: &Database) -> SkillsOverview {
    let dir = skills_dir(db);
    let catalog = dir.as_deref().map(load_catalog).unwrap_or_default();
    SkillsOverview {
        enabled: skills_enabled(db),
        dir,
        skills: catalog
            .skills
            .into_iter()
            .map(|s| SkillInfo {
                name: s.name,
                description: s.description,
                path: s.path,
            })
            .collect(),
        errors: catalog.errors,
    }
}

/// Create the skills folder if needed and return it — what Settings' "Open
/// folder" button reveals.
pub fn ensure_skills_dir(db: &Database) -> crate::models::error::Result<PathBuf> {
    let dir = skills_dir(db)
        .ok_or_else(|| crate::models::error::AppError::IoError("this install has no data folder".to_string()))?;
    std::fs::create_dir_all(&dir).map_err(|e| {
        crate::models::error::AppError::IoError(format!("could not create the skills folder '{}': {e}", dir.display()))
    })?;
    Ok(dir)
}

#[cfg(test)]
mod tests;
