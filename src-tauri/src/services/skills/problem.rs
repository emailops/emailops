//! Why a skill could not be loaded, saved, created or deleted.
//!
//! Every problem has a stable `code()` and structured `params()` so the UI
//! translates it (`errors:codes.<code>` on the frontend), and an English
//! `Display` that stays the fallback, the CLI text and the log line.

use std::collections::BTreeMap;

use super::{MAX_BODY_CHARS, MAX_DESCRIPTION_CHARS, MAX_NAME_CHARS};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SkillProblem {
    #[error("SKILL.md must start with a `---` frontmatter block holding `name` and `description`")]
    NoFrontmatter,
    #[error("the frontmatter block is never closed with `---`")]
    UnclosedFrontmatter,
    #[error("the frontmatter's closing `---` must be on its own line")]
    FenceNotOnOwnLine,
    #[error("invalid frontmatter: {0}")]
    InvalidFrontmatter(String),
    #[error("the frontmatter has no `name`")]
    NoName,
    #[error("a skill needs a name")]
    EmptyName,
    #[error("`{0}` is not a valid skill name: use lowercase letters, digits and single hyphens, at most {MAX_NAME_CHARS} characters")]
    InvalidName(String),
    #[error("`name: {name}` must match its folder name `{folder}` (rename one of them)")]
    NameMismatch { name: String, folder: String },
    #[error("the frontmatter has no `description` (say what the skill does and when to use it)")]
    NoDescription,
    #[error("the description is longer than {MAX_DESCRIPTION_CHARS} characters")]
    DescriptionTooLong,
    #[error("the skill has no instructions after the frontmatter")]
    NoBody,
    #[error(
        "the instructions are longer than {MAX_BODY_CHARS} characters; shorten them so they fit the model's context"
    )]
    BodyTooLong,
    #[error("{0}/SKILL.md changed on disk since you opened it; reload it before saving")]
    ChangedOnDisk(String),
    #[error("a skill named {0} already exists; pick another name")]
    AlreadyExists(String),
}

impl SkillProblem {
    /// Stable translation key suffix; renaming one breaks translations.
    pub fn code(&self) -> &'static str {
        match self {
            SkillProblem::NoFrontmatter => "skill_no_frontmatter",
            SkillProblem::UnclosedFrontmatter => "skill_unclosed_frontmatter",
            SkillProblem::FenceNotOnOwnLine => "skill_fence_not_on_own_line",
            SkillProblem::InvalidFrontmatter(_) => "skill_invalid_frontmatter",
            SkillProblem::NoName => "skill_no_name",
            SkillProblem::EmptyName => "skill_empty_name",
            SkillProblem::InvalidName(_) => "skill_invalid_name",
            SkillProblem::NameMismatch { .. } => "skill_name_mismatch",
            SkillProblem::NoDescription => "skill_no_description",
            SkillProblem::DescriptionTooLong => "skill_description_too_long",
            SkillProblem::NoBody => "skill_no_body",
            SkillProblem::BodyTooLong => "skill_body_too_long",
            SkillProblem::ChangedOnDisk(_) => "skill_changed_on_disk",
            SkillProblem::AlreadyExists(_) => "skill_already_exists",
        }
    }

    /// Values the translated message interpolates.
    pub fn params(&self) -> BTreeMap<&'static str, String> {
        let mut p = BTreeMap::new();
        match self {
            SkillProblem::InvalidFrontmatter(detail) => {
                p.insert("detail", detail.clone());
            }
            SkillProblem::InvalidName(name) | SkillProblem::ChangedOnDisk(name) | SkillProblem::AlreadyExists(name) => {
                p.insert("name", name.clone());
            }
            SkillProblem::NameMismatch { name, folder } => {
                p.insert("name", name.clone());
                p.insert("folder", folder.clone());
            }
            SkillProblem::DescriptionTooLong => {
                p.insert("max", MAX_DESCRIPTION_CHARS.to_string());
            }
            SkillProblem::BodyTooLong => {
                p.insert("max", MAX_BODY_CHARS.to_string());
            }
            SkillProblem::NoFrontmatter
            | SkillProblem::UnclosedFrontmatter
            | SkillProblem::FenceNotOnOwnLine
            | SkillProblem::NoName
            | SkillProblem::EmptyName
            | SkillProblem::NoDescription
            | SkillProblem::NoBody => {}
        }
        p
    }
}
