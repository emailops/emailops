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
                    "description": "Name of the skill to load."
                }
            },
            "required": ["name"]
        })
    }

    /// The catalog goes in the `name` description and the names become an
    /// `enum`, so a model with constrained decoding cannot invent one.
    fn parameters_schema_for(&self, db: &Database) -> Value {
        let catalog = skills::catalog_for(db);
        let (lines, names) = skills::render_catalog(&catalog.skills);
        if names.is_empty() {
            return self.parameters_schema();
        }
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "enum": names,
                    "description": format!("Name of the skill to load. Available skills:\n{lines}")
                }
            },
            "required": ["name"]
        })
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
        let catalog = skills::catalog_for(ctx.db);
        Ok(ToolOutput::text(match catalog.get(&requested) {
            Some(skill) => skills::render_skill_block(skill),
            None => format!(
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
    fn schema_advertises_the_catalog_and_restricts_names() {
        let (_tmp, db) = db_with_skills(&[
            ("weekly-summary", "Weekly recap by client.", "Group by client."),
            ("vendor-reply", "Reply to vendor quotes.", "Be brief."),
        ]);
        let schema = LoadSkillTool.parameters_schema_for(&db);
        let name = &schema["properties"]["name"];
        assert_eq!(name["enum"], json!(["vendor-reply", "weekly-summary"]));
        let desc = name["description"].as_str().unwrap();
        assert!(desc.contains("- vendor-reply: Reply to vendor quotes."), "{desc}");
        assert!(desc.contains("- weekly-summary: Weekly recap by client."), "{desc}");
        // The body never rides in the schema — that is the point.
        assert!(!desc.contains("Group by client."), "{desc}");
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
}
