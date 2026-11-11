//! `load_skill` — reads a reference file of a skill the user invoked.
//!
//! A skill runs only through `/name` (`services::skills`): its instructions
//! reach the model in that turn's final user message, so this tool never
//! returns them. It exists for the `.md` / `.txt` files some skills keep next to
//! their `SKILL.md`, which are too long to send on every use. It is hidden unless
//! an enabled skill has such a file, so an install with none pays nothing in the
//! prompt, and it depends only on the folder, so the system prefix stays
//! byte-identical turn to turn and the KV-prefix cache keeps working.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolCtx, ToolError, ToolOutput};
use crate::db::Database;
use crate::services::skills;

pub struct LoadSkillTool;

/// Names of the enabled skills that have reference files, sorted.
fn names_with_files(catalog: &skills::SkillCatalog) -> Vec<&str> {
    catalog
        .skills
        .iter()
        .filter(|s| !s.files.is_empty())
        .map(|s| s.name.as_str())
        .collect()
}

#[async_trait]
impl Tool for LoadSkillTool {
    fn name(&self) -> &'static str {
        "load_skill"
    }

    fn description(&self) -> &'static str {
        "Read one of the reference files listed by a skill the user invoked in this request. Call it only when the skill's instructions need that file."
    }

    fn prompt_summary(&self) -> &'static str {
        "read a reference file listed by a skill the user invoked in this request."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name of the skill."
                },
                "file": {
                    "type": "string",
                    "description": "One of the skill's reference files, as listed in its instructions."
                }
            },
            "required": ["name", "file"]
        })
    }

    /// The names become an `enum`, so a model with constrained decoding
    /// cannot invent one.
    fn parameters_schema_for(&self, db: &Database) -> Value {
        let catalog = skills::catalog_for(db);
        let mut schema = self.parameters_schema();
        let names = names_with_files(&catalog);
        if !names.is_empty() {
            schema["properties"]["name"]["enum"] = json!(names);
        }
        schema
    }

    /// Hidden unless the feature is on AND an enabled skill has reference
    /// files: every other install pays nothing in the prompt.
    fn is_available(&self, db: &Database) -> bool {
        !names_with_files(&skills::catalog_for(db)).is_empty()
    }

    async fn execute(&self, ctx: &ToolCtx<'_>, args: Value) -> Result<ToolOutput, ToolError> {
        let requested = args
            .get("name")
            .and_then(Value::as_str)
            .map(|s| s.trim().trim_start_matches('/').to_ascii_lowercase())
            .unwrap_or_default();
        let file = args
            .get("file")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        let catalog = skills::catalog_for(ctx.db);
        Ok(ToolOutput::text(match catalog.get(&requested) {
            Some(skill) => match skills::read_reference(skill, file) {
                Ok(text) => format!(
                    "<skill-file skill=\"{}\" path=\"{file}\">\n{text}\n</skill-file>",
                    skill.name
                ),
                Err(msg) => msg,
            },
            None => format!(
                "No skill named \"{requested}\". Skills with reference files: {}.",
                names_with_files(&catalog).join(", ")
            ),
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn db_with_skills(skills: &[(&str, &str, &str)]) -> (tempfile::TempDir, Arc<Database>) {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::new(tmp.path().to_path_buf()).unwrap());
        db.set_preference(skills::SKILLS_ENABLED_PREF, "true").unwrap();
        for (name, description, body) in skills {
            let dir = tmp.path().join(skills::SKILLS_DIR).join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join(skills::SKILL_FILE),
                format!("---\nname: {name}\ndescription: {description}\n---\n{body}"),
            )
            .unwrap();
        }
        (tmp, db)
    }

    fn ctx(db: &Arc<Database>) -> ToolCtx<'_> {
        ToolCtx {
            db,
            account_id: "acc",
            categories: &[],
            page: None,
        }
    }

    fn add_reference(tmp: &tempfile::TempDir, skill: &str) {
        let refs = tmp.path().join(skills::SKILLS_DIR).join(skill).join("references");
        std::fs::create_dir_all(&refs).unwrap();
        std::fs::write(refs.join("tone.md"), "Always formal.").unwrap();
    }

    #[test]
    fn hidden_when_there_are_no_skills() {
        let (_tmp, db) = db_with_skills(&[]);
        assert!(!LoadSkillTool.is_available(&db));
        let test_db = Database::new_for_testing().unwrap();
        assert!(!LoadSkillTool.is_available(&test_db));
    }

    #[test]
    fn hidden_while_no_enabled_skill_has_a_reference_file() {
        let (_tmp, db) = db_with_skills(&[("weekly-summary", "Weekly recap.", "Group by client.")]);
        assert!(!LoadSkillTool.is_available(&db));
        skills::set_bundled_skill_enabled(&db, "inbox-triage", true).unwrap();
        assert!(!LoadSkillTool.is_available(&db));
    }

    #[test]
    fn available_once_a_skill_has_a_reference_file() {
        let (tmp, db) = db_with_skills(&[("vendor-reply", "Reply to vendors.", "See references.")]);
        add_reference(&tmp, "vendor-reply");
        assert!(LoadSkillTool.is_available(&db));
        db.set_preference(skills::SKILLS_ENABLED_PREF, "false").unwrap();
        assert!(!LoadSkillTool.is_available(&db));
    }

    #[test]
    fn hidden_when_the_only_skill_with_files_is_switched_off() {
        let (tmp, db) = db_with_skills(&[("vendor-reply", "Reply to vendors.", "See references.")]);
        add_reference(&tmp, "vendor-reply");
        skills::set_skill_enabled(&db, "vendor-reply", false).unwrap();
        assert!(!LoadSkillTool.is_available(&db));
    }

    #[test]
    fn schema_names_only_the_skills_with_reference_files() {
        let (tmp, db) = db_with_skills(&[
            ("weekly-summary", "Weekly recap by client.", "Group by client."),
            ("vendor-reply", "Reply to vendor quotes.", "Be brief."),
        ]);
        add_reference(&tmp, "vendor-reply");
        let schema = LoadSkillTool.parameters_schema_for(&db);
        assert_eq!(schema["properties"]["name"]["enum"], json!(["vendor-reply"]));
        assert_eq!(schema["required"], json!(["name", "file"]));
        assert!(!schema.to_string().contains("Reply to vendor quotes."), "{schema}");
    }

    #[test]
    fn schema_is_byte_identical_across_calls() {
        // The schema is part of the cached system prefix: any drift between
        // turns would force a cold prefill.
        let (tmp, db) = db_with_skills(&[("b", "Second.", "x"), ("a", "First.", "y")]);
        add_reference(&tmp, "a");
        add_reference(&tmp, "b");
        assert_eq!(
            LoadSkillTool.parameters_schema_for(&db).to_string(),
            LoadSkillTool.parameters_schema_for(&db).to_string()
        );
    }

    #[tokio::test]
    async fn never_returns_a_skills_instructions() {
        // A skill runs only when the user types `/name`: the model cannot pull
        // one in by itself.
        let (tmp, db) = db_with_skills(&[("vendor-reply", "Reply to vendors.", "Always ask for the PO number.")]);
        add_reference(&tmp, "vendor-reply");
        let out = LoadSkillTool
            .execute(&ctx(&db), json!({"name": "vendor-reply"}))
            .await
            .unwrap();
        assert!(!out.text.contains("Always ask for the PO number."), "{}", out.text);
        assert!(out.text.contains("references/tone.md"), "{}", out.text);
    }

    #[tokio::test]
    async fn tolerates_a_slash_and_case_in_the_name() {
        let (tmp, db) = db_with_skills(&[("vendor-reply", "Reply to vendors.", "Body.")]);
        add_reference(&tmp, "vendor-reply");
        let out = LoadSkillTool
            .execute(
                &ctx(&db),
                json!({"name": "/Vendor-Reply", "file": "references/tone.md"}),
            )
            .await
            .unwrap();
        assert!(out.text.contains("Always formal."), "{}", out.text);
    }

    #[tokio::test]
    async fn an_unknown_name_lists_the_skills_with_files() {
        let (tmp, db) = db_with_skills(&[("vendor-reply", "Reply to vendors.", "Body.")]);
        add_reference(&tmp, "vendor-reply");
        let out = LoadSkillTool
            .execute(&ctx(&db), json!({"name": "nope", "file": "x.md"}))
            .await
            .unwrap();
        assert!(out.text.contains("No skill named \"nope\""), "{}", out.text);
        assert!(out.text.contains("vendor-reply"));
    }

    #[tokio::test]
    async fn reads_a_listed_reference_file() {
        let (tmp, db) = db_with_skills(&[("vendor-reply", "Reply to vendors.", "See references.")]);
        add_reference(&tmp, "vendor-reply");
        let out = LoadSkillTool
            .execute(&ctx(&db), json!({"name": "vendor-reply", "file": "references/tone.md"}))
            .await
            .unwrap();
        assert!(out.text.contains("Always formal."), "{}", out.text);
        let refused = LoadSkillTool
            .execute(&ctx(&db), json!({"name": "vendor-reply", "file": "../../emailops.db"}))
            .await
            .unwrap();
        assert!(refused.text.contains("has no file"), "{}", refused.text);
    }
}
