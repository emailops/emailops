//! `load_skill` — the model's way into the user's skills (`services::skills`).
//!
//! Progressive disclosure: the catalog (`- name: description`, one line per
//! skill) rides in this tool's `name` parameter description, so it lands in the
//! system prompt's `<tools>` block; a skill's body is returned only when the
//! model calls the tool. The catalog changes only when the skills folder does,
//! so the system prefix stays byte-identical turn to turn and the KV-prefix
//! cache keeps working.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolCtx, ToolError, ToolOutput};
use crate::db::Database;
use crate::services::skills;

pub struct LoadSkillTool;

#[async_trait]
impl Tool for LoadSkillTool {
    fn name(&self) -> &'static str {
        "load_skill"
    }

    fn description(&self) -> &'static str {
        "Load the instructions of one of the user's skills (saved procedures such as how to write a weekly summary or reply to a vendor). Call it FIRST when the request matches a skill's description, then follow the instructions it returns, using the other tools as they say."
    }

    fn prompt_summary(&self) -> &'static str {
        "load one of the user's saved skills when the request matches its description; then follow what it returns."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name of the skill to load (see Skills in the system prompt)."
                },
                "file": {
                    "type": "string",
                    "description": "Optional: one of the skill's reference files, as listed when the skill was loaded."
                }
            },
            "required": ["name"]
        })
    }

    /// The names become an `enum`, so a model with constrained decoding
    /// cannot invent one. The descriptions ride once, in `prompt_appendix`.
    fn parameters_schema_for(&self, db: &Database) -> Value {
        let catalog = skills::catalog_for(db);
        let (_, names) = skills::render_catalog(&catalog.skills);
        let mut schema = self.parameters_schema();
        if !names.is_empty() {
            schema["properties"]["name"]["enum"] = json!(names);
        }
        schema
    }

    /// Hermes-style skills index: the catalog plus the instruction to load a
    /// matching skill before answering. Depends only on the skills folder, so
    /// the system prefix stays byte-identical turn to turn.
    fn prompt_appendix(&self, db: &Database) -> Option<String> {
        skills::render_skills_index(&skills::catalog_for(db).skills)
    }

    /// Hidden unless the feature is on AND at least one valid skill exists: an
    /// install without skills pays nothing in the prompt.
    fn is_available(&self, db: &Database) -> bool {
        !skills::catalog_for(db).skills.is_empty()
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
            .filter(|f| !f.is_empty());
        let catalog = skills::catalog_for(ctx.db);
        Ok(ToolOutput::text(match (catalog.get(&requested), file) {
            (Some(skill), None) => skills::render_skill_block(skill),
            (Some(skill), Some(file)) => match skills::read_reference(skill, file) {
                Ok(text) => format!(
                    "<skill-file skill=\"{}\" path=\"{file}\">\n{text}\n</skill-file>",
                    skill.name
                ),
                Err(msg) => msg,
            },
            (None, _) => format!(
                "No skill named \"{requested}\". Available skills: {}.",
                catalog.names().join(", ")
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

    #[test]
    fn hidden_when_there_are_no_skills() {
        let (_tmp, db) = db_with_skills(&[]);
        assert!(!LoadSkillTool.is_available(&db));
        let test_db = Database::new_for_testing().unwrap();
        assert!(!LoadSkillTool.is_available(&test_db));
    }

    #[test]
    fn hidden_when_the_feature_is_off() {
        let (_tmp, db) = db_with_skills(&[("weekly-summary", "Weekly recap.", "Group by client.")]);
        assert!(LoadSkillTool.is_available(&db));
        db.set_preference(skills::SKILLS_ENABLED_PREF, "false").unwrap();
        assert!(!LoadSkillTool.is_available(&db));
    }

    #[test]
    fn schema_restricts_names_and_leaves_the_catalog_to_the_index() {
        let (_tmp, db) = db_with_skills(&[
            ("weekly-summary", "Weekly recap by client.", "Group by client."),
            ("vendor-reply", "Reply to vendor quotes.", "Be brief."),
        ]);
        let schema = LoadSkillTool.parameters_schema_for(&db);
        assert_eq!(
            schema["properties"]["name"]["enum"],
            json!(["vendor-reply", "weekly-summary"])
        );
        assert_eq!(schema["required"], json!(["name"]));
        assert!(schema["properties"]["file"].is_object(), "{schema}");
        // Descriptions ride once, in the system-prompt index, not again here.
        assert!(!schema.to_string().contains("Weekly recap by client."), "{schema}");
    }

    #[test]
    fn the_prompt_appendix_is_the_skills_index() {
        let (_tmp, db) = db_with_skills(&[("vendor-reply", "Reply to vendor quotes.", "Be brief.")]);
        let appendix = LoadSkillTool.prompt_appendix(&db).unwrap();
        assert!(
            appendix.contains("- vendor-reply: Reply to vendor quotes."),
            "{appendix}"
        );
        assert!(appendix.contains("FIRST"), "{appendix}");
        // The body stays out of every-turn text — that is the point.
        assert!(!appendix.contains("Be brief."), "{appendix}");
    }

    #[test]
    fn schema_is_byte_identical_across_calls() {
        // The schema is part of the cached system prefix: any drift between
        // turns would force a cold prefill.
        let (_tmp, db) = db_with_skills(&[("b", "Second.", "x"), ("a", "First.", "y")]);
        assert_eq!(
            LoadSkillTool.parameters_schema_for(&db).to_string(),
            LoadSkillTool.parameters_schema_for(&db).to_string()
        );
    }

    #[tokio::test]
    async fn returns_the_skill_body_for_a_known_name() {
        let (_tmp, db) = db_with_skills(&[("vendor-reply", "Reply to vendors.", "Always ask for the PO number.")]);
        let out = LoadSkillTool
            .execute(&ctx(&db), json!({"name": "vendor-reply"}))
            .await
            .unwrap();
        assert!(out.text.contains("<skill name=\"vendor-reply\">"), "{}", out.text);
        assert!(out.text.contains("Always ask for the PO number."));
    }

    #[tokio::test]
    async fn tolerates_a_slash_and_case_in_the_name() {
        let (_tmp, db) = db_with_skills(&[("vendor-reply", "Reply to vendors.", "Body.")]);
        let out = LoadSkillTool
            .execute(&ctx(&db), json!({"name": "/Vendor-Reply"}))
            .await
            .unwrap();
        assert!(out.text.contains("Body."), "{}", out.text);
    }

    #[tokio::test]
    async fn an_unknown_name_lists_the_available_skills() {
        let (_tmp, db) = db_with_skills(&[("vendor-reply", "Reply to vendors.", "Body.")]);
        let out = LoadSkillTool.execute(&ctx(&db), json!({"name": "nope"})).await.unwrap();
        assert!(out.text.contains("No skill named \"nope\""), "{}", out.text);
        assert!(out.text.contains("vendor-reply"));
    }

    #[tokio::test]
    async fn reads_a_listed_reference_file() {
        let (tmp, db) = db_with_skills(&[("vendor-reply", "Reply to vendors.", "See references.")]);
        let refs = tmp.path().join(skills::SKILLS_DIR).join("vendor-reply/references");
        std::fs::create_dir_all(&refs).unwrap();
        std::fs::write(refs.join("tone.md"), "Always formal.").unwrap();
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
