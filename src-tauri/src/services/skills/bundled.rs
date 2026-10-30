//! Reviewed skills that ship inside the binary.
//!
//! The text lives in `src-tauri/bundled-skills/` (copied from `app-skills/` of
//! `emailops-skills`, see `SOURCE` there) and is embedded here, so listing and
//! reading one needs neither the network nor the skills folder. A bundled skill
//! is used in place: it has its own switch, off by default, and nothing is
//! written under `skills/` unless the user asks for an editable copy.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::Serialize;

use super::{
    parse_skill_md, set_skill_enabled, skill_folder, write_atomically, Skill, SkillCatalog, SkillLoadError,
    SkillProblem, SKILL_FILE,
};
use crate::db::Database;
use crate::models::error::{AppError, Result};

/// Preference holding the bundled skills the user switched on (JSON array of
/// names). An allow list, unlike `skills_disabled`: they are off by default.
pub const SKILLS_BUNDLED_ON_PREF: &str = "skills_bundled_on";

/// Name and `SKILL.md` text of every bundled skill, sorted by name. A test
/// fails if this list and the folder disagree.
const BUNDLED: &[(&str, &str)] = &[
    (
        "follow-up-check",
        include_str!("../../../bundled-skills/follow-up-check/SKILL.md"),
    ),
    (
        "inbox-triage",
        include_str!("../../../bundled-skills/inbox-triage/SKILL.md"),
    ),
];

/// The embedded `SKILL.md` of a bundled skill.
pub fn bundled_source(name: &str) -> Option<&'static str> {
    BUNDLED.iter().find(|(n, _)| *n == name).map(|(_, text)| *text)
}

/// Parse every bundled skill. They carry no path and no reference files.
pub fn load_bundled() -> SkillCatalog {
    let mut catalog = SkillCatalog::default();
    for (name, text) in BUNDLED {
        match parse_skill_md(text, name) {
            Ok(skill) => catalog.skills.push(skill),
            Err(problem) => catalog.errors.push(SkillLoadError::new(
                PathBuf::from("bundled-skills").join(name).join(SKILL_FILE),
                &problem,
            )),
        }
    }
    catalog
}

/// Names of the bundled skills the user switched on.
pub fn bundled_skills_on(db: &Database) -> BTreeSet<String> {
    db.get_preference(SKILLS_BUNDLED_ON_PREF)
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}

/// Switch one bundled skill on or off for the chat.
pub fn set_bundled_skill_enabled(db: &Database, name: &str, enabled: bool) -> Result<()> {
    if bundled_source(name).is_none() {
        return Err(AppError::NotFound(format!("bundled skill {name}")));
    }
    // Read-modify-write of one preference, as in `set_skill_enabled`.
    static TOGGLE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = TOGGLE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut on = bundled_skills_on(db);
    if enabled {
        on.insert(name.to_string());
    } else {
        on.remove(name);
    }
    let json = serde_json::to_string(&on)
        .map_err(|e| AppError::IoError(format!("could not store the bundled skills: {e}")))?;
    db.set_preference(SKILLS_BUNDLED_ON_PREF, &json)
}

/// The names the user's own folder claims, loadable or not: a typo in the
/// user's `inbox-triage/SKILL.md` must show as an error, not silently bring
/// the bundled text back in its place.
pub(super) fn user_names(catalog: &SkillCatalog) -> BTreeSet<String> {
    let broken = catalog
        .errors
        .iter()
        .filter_map(|e| Some(e.path.parent()?.file_name()?.to_string_lossy().into_owned()));
    catalog.skills.iter().map(|s| s.name.clone()).chain(broken).collect()
}

/// Add the switched-on bundled skills to the user's `catalog`. `taken` is
/// [`user_names`], which win. With nothing switched on the catalog is left as
/// it is, so the prompts stay what they were.
pub(super) fn merge_active(catalog: &mut SkillCatalog, on: &BTreeSet<String>, taken: &BTreeSet<String>) {
    if on.is_empty() {
        return;
    }
    let active = load_bundled()
        .skills
        .into_iter()
        .filter(|s| on.contains(&s.name) && !taken.contains(&s.name));
    catalog.skills.extend(active);
    catalog.skills.sort_by(|a, b| a.name.cmp(&b.name));
}

/// One bundled skill as listed in Settings, the Skills view and the CLI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundledSkillInfo {
    pub name: String,
    pub description: String,
    /// The bundled skill's own switch.
    pub enabled: bool,
    /// A skill of the user's has this name, so that one is used instead.
    pub shadowed: bool,
}

/// The bundled skills for the listing, on or off.
pub(super) fn list_bundled(
    skills: Vec<Skill>,
    on: &BTreeSet<String>,
    taken: &BTreeSet<String>,
) -> Vec<BundledSkillInfo> {
    skills
        .into_iter()
        .map(|s| BundledSkillInfo {
            enabled: on.contains(&s.name),
            shadowed: taken.contains(&s.name),
            name: s.name,
            description: s.description,
        })
        .collect()
}

/// The full text of a bundled skill, for reading before switching it on.
pub fn read_bundled_source(name: &str) -> Result<&'static str> {
    bundled_source(name).ok_or_else(|| AppError::NotFound(format!("bundled skill {name}")))
}

/// Create a normal user skill from a bundled one, for the user to edit. The
/// copy keeps the bundled skill's switch: copying one that is off must not put
/// it in the prompt.
pub fn copy_bundled_skill(db: &Database, name: &str) -> Result<()> {
    let text = read_bundled_source(name)?;
    let folder = skill_folder(db, name)?;
    if folder.exists() {
        return Err(SkillProblem::AlreadyExists(name.to_string()).into());
    }
    std::fs::create_dir_all(&folder)
        .map_err(|e| AppError::IoError(format!("could not create '{}': {e}", folder.display())))?;
    write_atomically(&folder.join(SKILL_FILE), text)?;
    if !bundled_skills_on(db).contains(name) {
        set_skill_enabled(db, name, false)?;
    }
    Ok(())
}
