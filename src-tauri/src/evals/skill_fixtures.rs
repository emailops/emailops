//! Skills a chat eval case brings with it (`skills:` in the case YAML).
//!
//! Skills live on disk next to the database (`services::skills::skills_dir`),
//! so a case that exercises them writes its skill folders there for the length
//! of the case. The runner normally works on a temp copy of the DB, but it can
//! also run in place on the developer's own data dir — so the guard refuses to
//! overwrite a skill folder that already exists, and on drop removes only the
//! folders it created.

use std::path::PathBuf;

use serde::Deserialize;

use crate::db::Database;
use crate::services::skills;

use super::{EvalError, EvalResult};

/// One skill a case installs for its run.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct SkillFixture {
    pub name: String,
    pub description: String,
    pub body: String,
}

/// Removes the fixture folders it created when dropped, and puts the
/// `skills_enabled` and bundled-skill preferences back the way it found them.
pub struct SkillFixtureGuard<'a> {
    db: &'a Database,
    created: Vec<PathBuf>,
    /// The preference before the case switched the (experimental, default
    /// off) feature on; `None` when it was unset.
    enabled_before: Option<String>,
    /// The bundled skills' preference before the case; `None` when unset.
    bundled_before: Option<String>,
}

impl<'a> SkillFixtureGuard<'a> {
    /// Write `fixtures` into the DB's skills folder and switch the `bundled`
    /// skills on. `Ok(None)` when the case has neither.
    pub fn install(db: &'a Database, fixtures: &[SkillFixture], bundled: &[String]) -> EvalResult<Option<Self>> {
        if fixtures.is_empty() && bundled.is_empty() {
            return Ok(None);
        }
        if let Some(unknown) = bundled.iter().find(|n| skills::bundled_source(n).is_none()) {
            return Err(EvalError::Config(format!(
                "case names unknown bundled skill `{unknown}`"
            )));
        }
        let dir = skills::skills_dir(db)
            .ok_or_else(|| EvalError::Config("case has `skills:` but the eval DB has no data dir".into()))?;
        // Check every folder before writing any, so a clash leaves nothing
        // half-installed.
        if let Some(clash) = fixtures.iter().find(|f| dir.join(&f.name).exists()) {
            return Err(EvalError::Config(format!(
                "case skill `{}` would overwrite an existing skill at {}",
                clash.name,
                dir.join(&clash.name).display()
            )));
        }
        // Built before the first write: if a later write fails, dropping it
        // removes what was already created.
        let mut guard = Self {
            db,
            created: Vec::new(),
            enabled_before: db
                .get_preference(skills::SKILLS_ENABLED_PREF)
                .map_err(|e| EvalError::Config(format!("cannot read {}: {e}", skills::SKILLS_ENABLED_PREF)))?,
            bundled_before: db
                .get_preference(skills::SKILLS_BUNDLED_ON_PREF)
                .map_err(|e| EvalError::Config(format!("cannot read {}: {e}", skills::SKILLS_BUNDLED_ON_PREF)))?,
        };
        db.set_preference(skills::SKILLS_ENABLED_PREF, "true")
            .map_err(|e| EvalError::Config(format!("cannot enable skills for the case: {e}")))?;
        for name in bundled {
            skills::set_bundled_skill_enabled(db, name, true)
                .map_err(|e| EvalError::Config(format!("cannot switch on bundled skill {name}: {e}")))?;
        }
        for f in fixtures {
            let folder = dir.join(&f.name);
            std::fs::create_dir_all(&folder)?;
            guard.created.push(folder.clone());
            let text = format!(
                "---\nname: {}\ndescription: {}\n---\n{}\n",
                f.name,
                serde_json::to_string(&f.description)?,
                f.body
            );
            std::fs::write(folder.join(skills::SKILL_FILE), text)?;
        }
        Ok(Some(guard))
    }
}

// Manual: `Database` is not `Debug`.
impl std::fmt::Debug for SkillFixtureGuard<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkillFixtureGuard")
            .field("created", &self.created)
            .field("enabled_before", &self.enabled_before)
            .field("bundled_before", &self.bundled_before)
            .finish()
    }
}

impl Drop for SkillFixtureGuard<'_> {
    fn drop(&mut self) {
        let restored = match &self.enabled_before {
            Some(v) => self.db.set_preference(skills::SKILLS_ENABLED_PREF, v),
            None => self.db.delete_preference(skills::SKILLS_ENABLED_PREF),
        };
        if let Err(e) = restored {
            eprintln!("[eval] could not restore {}: {e}", skills::SKILLS_ENABLED_PREF);
        }
        let restored = match &self.bundled_before {
            Some(v) => self.db.set_preference(skills::SKILLS_BUNDLED_ON_PREF, v),
            None => self.db.delete_preference(skills::SKILLS_BUNDLED_ON_PREF),
        };
        if let Err(e) = restored {
            eprintln!("[eval] could not restore {}: {e}", skills::SKILLS_BUNDLED_ON_PREF);
        }
        for folder in &self.created {
            if let Err(e) = std::fs::remove_dir_all(folder) {
                eprintln!("[eval] could not remove case skill {}: {e}", folder.display());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> SkillFixture {
        SkillFixture {
            name: name.into(),
            description: format!("{name} description."),
            body: "Follow these steps.".into(),
        }
    }

    #[test]
    fn no_fixtures_installs_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::new(tmp.path().to_path_buf()).unwrap();
        assert!(SkillFixtureGuard::install(&db, &[], &[]).unwrap().is_none());
        assert!(!tmp.path().join(skills::SKILLS_DIR).exists());
    }

    #[test]
    fn installs_loadable_skills_and_removes_them_on_drop() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::new(tmp.path().to_path_buf()).unwrap();
        let guard = SkillFixtureGuard::install(&db, &[fixture("alpha"), fixture("beta")], &[]).unwrap();
        let catalog = skills::catalog_for(&db);
        assert_eq!(catalog.names(), vec!["alpha", "beta"]);
        assert!(catalog.errors.is_empty(), "{:?}", catalog.errors);
        drop(guard);
        assert!(skills::catalog_for(&db).skills.is_empty());
    }

    #[test]
    fn switches_a_bundled_skill_on_for_the_case_and_restores_it() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::new(tmp.path().to_path_buf()).unwrap();
        let guard = SkillFixtureGuard::install(&db, &[], &["inbox-triage".to_string()]).unwrap();
        assert_eq!(skills::catalog_for(&db).names(), vec!["inbox-triage"]);
        drop(guard);
        assert!(!skills::skills_enabled(&db));
        assert_eq!(db.get_preference(skills::SKILLS_BUNDLED_ON_PREF).unwrap(), None);
    }

    #[test]
    fn a_case_naming_an_unknown_bundled_skill_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::new(tmp.path().to_path_buf()).unwrap();
        let err = SkillFixtureGuard::install(&db, &[], &["not-bundled".to_string()]).unwrap_err();
        assert!(err.to_string().contains("not-bundled"), "{err}");
        assert!(!skills::skills_enabled(&db));
    }

    #[test]
    fn switches_the_feature_on_for_the_case_and_restores_it() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::new(tmp.path().to_path_buf()).unwrap();
        assert!(!skills::skills_enabled(&db));
        let guard = SkillFixtureGuard::install(&db, &[fixture("alpha")], &[]).unwrap();
        assert!(skills::skills_enabled(&db));
        drop(guard);
        assert!(!skills::skills_enabled(&db));
    }

    #[test]
    fn refuses_to_overwrite_an_existing_skill_and_leaves_it_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::new(tmp.path().to_path_buf()).unwrap();
        let mine = tmp.path().join(skills::SKILLS_DIR).join("alpha");
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::write(mine.join(skills::SKILL_FILE), "the user's own skill").unwrap();

        let err = SkillFixtureGuard::install(&db, &[fixture("beta"), fixture("alpha")], &[]).unwrap_err();
        assert!(err.to_string().contains("alpha"), "{err}");
        assert_eq!(
            std::fs::read_to_string(mine.join(skills::SKILL_FILE)).unwrap(),
            "the user's own skill"
        );
        // Nothing half-installed is left behind.
        assert!(!tmp.path().join(skills::SKILLS_DIR).join("beta").exists());
    }

    #[test]
    fn errors_without_a_data_dir() {
        let db = Database::new_for_testing().unwrap();
        assert!(SkillFixtureGuard::install(&db, &[fixture("alpha")], &[]).is_err());
    }
}
