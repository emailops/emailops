use std::fs;

use super::*;

fn skill_md(name: &str, description: &str, body: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\n{body}")
}

fn skill(name: &str, description: &str) -> Skill {
    Skill {
        name: name.to_string(),
        description: description.to_string(),
        body: "Do the thing.".to_string(),
        path: PathBuf::new(),
    }
}

// ── parse_skill_md ─────────────────────────────────────────────────────

#[test]
fn parses_name_description_and_body() {
    let text = skill_md(
        "weekly-summary",
        "Summarise the week. Use for weekly recaps.",
        "\n# Steps\n\n1. Group by client.\n",
    );
    let s = parse_skill_md(&text, "weekly-summary").unwrap();
    assert_eq!(s.name, "weekly-summary");
    assert_eq!(s.description, "Summarise the week. Use for weekly recaps.");
    assert_eq!(s.body, "# Steps\n\n1. Group by client.");
}

#[test]
fn accepts_quoted_and_folded_yaml_descriptions() {
    let text = "---\nname: vendor-reply\ndescription: >\n  Reply to a vendor quote.\n  Use when answering suppliers.\nextra: ignored\n---\nBe brief.";
    let s = parse_skill_md(text, "vendor-reply").unwrap();
    assert_eq!(s.description, "Reply to a vendor quote. Use when answering suppliers.");

    let quoted = "---\nname: \"vendor-reply\"\ndescription: 'Reply: to vendors'\n---\nBe brief.";
    assert_eq!(
        parse_skill_md(quoted, "vendor-reply").unwrap().description,
        "Reply: to vendors"
    );
}

#[test]
fn accepts_crlf_line_endings_and_a_bom() {
    let text = "\u{feff}---\r\nname: crlf\r\ndescription: Windows file.\r\n---\r\nLine one.\r\n";
    let s = parse_skill_md(text, "crlf").unwrap();
    assert_eq!(s.description, "Windows file.");
    assert_eq!(s.body, "Line one.");
}

#[test]
fn rejects_a_file_without_frontmatter() {
    let err = parse_skill_md("# Just markdown", "x").unwrap_err();
    assert!(err.contains("frontmatter"), "{err}");
}

#[test]
fn rejects_unterminated_frontmatter() {
    let err = parse_skill_md("---\nname: x\ndescription: y\n", "x").unwrap_err();
    assert!(err.contains("frontmatter"), "{err}");
}

#[test]
fn rejects_missing_name_or_description() {
    let no_desc = "---\nname: x\n---\nbody";
    assert!(parse_skill_md(no_desc, "x").unwrap_err().contains("description"));
    let no_name = "---\ndescription: y\n---\nbody";
    assert!(parse_skill_md(no_name, "x").unwrap_err().contains("name"));
}

#[test]
fn rejects_invalid_names() {
    for bad in [
        "Weekly",
        "weekly summary",
        "-weekly",
        "weekly-",
        "we--ekly",
        "semana_ñ",
        "",
    ] {
        let text = skill_md(bad, "d", "b");
        assert!(parse_skill_md(&text, bad).is_err(), "{bad:?} should be rejected");
    }
    let long = "a".repeat(MAX_NAME_CHARS + 1);
    assert!(parse_skill_md(&skill_md(&long, "d", "b"), &long).is_err());
}

#[test]
fn rejects_a_name_that_does_not_match_its_folder() {
    let err = parse_skill_md(&skill_md("weekly", "d", "b"), "monthly").unwrap_err();
    assert!(err.contains("monthly"), "{err}");
}

#[test]
fn rejects_an_empty_body() {
    let err = parse_skill_md(&skill_md("x", "d", "  \n"), "x").unwrap_err();
    assert!(err.contains("instructions"), "{err}");
}

#[test]
fn rejects_an_overlong_description_or_body_instead_of_cutting_it() {
    let long_desc = "d".repeat(MAX_DESCRIPTION_CHARS + 1);
    assert!(parse_skill_md(&skill_md("x", &long_desc, "b"), "x").is_err());
    let long_body = "b".repeat(MAX_BODY_CHARS + 1);
    let err = parse_skill_md(&skill_md("x", "d", &long_body), "x").unwrap_err();
    assert!(err.contains(&MAX_BODY_CHARS.to_string()), "{err}");
}

#[test]
fn a_description_is_collapsed_to_one_line() {
    // The catalog is one line per skill; a multi-line description would
    // break that shape.
    let text = "---\nname: x\ndescription: |\n  line one\n  line two\n---\nbody";
    assert_eq!(parse_skill_md(text, "x").unwrap().description, "line one line two");
}

// ── plan_invocation ────────────────────────────────────────────────────

#[test]
fn a_leading_slash_name_invokes_the_skill() {
    let names = ["weekly-summary", "vendor-reply"];
    assert_eq!(
        plan_invocation("/vendor-reply the quote from ACME", &names),
        Some(Invocation {
            name: "vendor-reply".into(),
            rest: "the quote from ACME".into()
        })
    );
    assert_eq!(
        plan_invocation("  /weekly-summary  ", &names),
        Some(Invocation {
            name: "weekly-summary".into(),
            rest: String::new()
        })
    );
    assert_eq!(
        plan_invocation("/weekly-summary\nsolo clientes de Madrid", &names),
        Some(Invocation {
            name: "weekly-summary".into(),
            rest: "solo clientes de Madrid".into()
        })
    );
}

#[test]
fn the_skill_name_is_matched_case_insensitively() {
    assert_eq!(
        plan_invocation("/Weekly-Summary", &["weekly-summary"]).map(|i| i.name),
        Some("weekly-summary".to_string())
    );
}

#[test]
fn text_that_is_not_a_leading_known_skill_is_left_alone() {
    let names = ["weekly-summary"];
    for msg in [
        "weekly-summary please",
        "run /weekly-summary",
        "/weekly-summaryX",
        "/weekly-summary.",
        "/unknown do it",
        "/",
        "",
        "/2026 invoices",
        "¿qué pasó el 12/05?",
    ] {
        assert_eq!(plan_invocation(msg, &names), None, "{msg:?}");
    }
}

#[test]
fn no_skills_means_no_invocation() {
    assert_eq!(plan_invocation("/weekly-summary", &[]), None);
}

// ── render_catalog / render_skill_block ────────────────────────────────

#[test]
fn catalog_lists_one_line_per_skill_in_name_order() {
    let (text, names) = render_catalog(&[skill("b-skill", "Second."), skill("a-skill", "First.")]);
    assert_eq!(text, "- a-skill: First.\n- b-skill: Second.");
    assert_eq!(names, vec!["a-skill", "b-skill"]);
}

#[test]
fn catalog_stops_at_its_budget_and_only_names_what_it_lists() {
    let desc = "x".repeat(400);
    let skills: Vec<Skill> = (0..20).map(|i| skill(&format!("s{i:02}"), &desc)).collect();
    let (text, names) = render_catalog(&skills);
    assert!(text.chars().count() <= MAX_CATALOG_CHARS, "{}", text.len());
    assert!(!names.is_empty() && names.len() < 20);
    assert_eq!(text.lines().count(), names.len());
    assert_eq!(names[0], "s00");
}

#[test]
fn empty_catalog_renders_nothing() {
    assert_eq!(render_catalog(&[]), (String::new(), Vec::new()));
}

#[test]
fn skill_block_carries_the_name_and_the_body() {
    let s = skill("vendor-reply", "Reply to vendors.");
    let block = render_skill_block(&s);
    assert!(block.starts_with("<skill name=\"vendor-reply\">"), "{block}");
    assert!(block.contains("Do the thing."));
    assert!(block.trim_end().ends_with("</skill>"));
}

// ── load_catalog (executor) ────────────────────────────────────────────

fn write_skill(root: &Path, folder: &str, text: &str) {
    let dir = root.join(folder);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(SKILL_FILE), text).unwrap();
}

#[test]
fn a_missing_folder_is_an_empty_catalog() {
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(load_catalog(&tmp.path().join("nope")), SkillCatalog::default());
}

#[test]
fn loads_valid_skills_sorted_and_reports_broken_ones() {
    let tmp = tempfile::tempdir().unwrap();
    write_skill(tmp.path(), "zeta", &skill_md("zeta", "Last.", "Z body"));
    write_skill(tmp.path(), "alpha", &skill_md("alpha", "First.", "A body"));
    write_skill(tmp.path(), "broken", "no frontmatter here");
    // A folder without SKILL.md and a stray file are not skills and not errors.
    fs::create_dir_all(tmp.path().join("notes")).unwrap();
    fs::write(tmp.path().join("README.txt"), "hi").unwrap();

    let catalog = load_catalog(tmp.path());
    assert_eq!(catalog.names(), vec!["alpha", "zeta"]);
    assert_eq!(catalog.get("alpha").unwrap().body, "A body");
    assert!(catalog.get("alpha").unwrap().path.ends_with("alpha/SKILL.md"));
    assert_eq!(catalog.errors.len(), 1);
    assert!(catalog.errors[0].path.ends_with("broken/SKILL.md"));
}

#[test]
fn catalog_for_is_empty_when_the_feature_is_off_or_there_is_no_data_dir() {
    let db = Database::new_for_testing().unwrap();
    assert!(skills_dir(&db).is_none());
    assert_eq!(catalog_for(&db), SkillCatalog::default());
    assert!(skills_enabled(&db));
    db.set_preference(SKILLS_ENABLED_PREF, "false").unwrap();
    assert!(!skills_enabled(&db));
}

// ── plan_skill_turn ────────────────────────────────────────────────────

fn catalog_of(skills: Vec<Skill>) -> SkillCatalog {
    SkillCatalog {
        skills,
        errors: Vec::new(),
    }
}

#[test]
fn a_skill_turn_asks_the_rest_and_carries_the_block() {
    let catalog = catalog_of(vec![skill("vendor-reply", "Reply to vendors.")]);
    let turn = plan_skill_turn("/vendor-reply the ACME quote", &catalog).unwrap();
    assert_eq!(turn.skill, "vendor-reply");
    assert_eq!(turn.question, "the ACME quote");
    assert_eq!(turn.block, render_skill_block(&catalog.skills[0]));
}

#[test]
fn a_bare_invocation_asks_to_apply_the_skill() {
    // Retrieval, the planner and the conversation title all read the
    // question; an empty one would give them nothing to work with.
    let catalog = catalog_of(vec![skill("weekly-summary", "Weekly recap.")]);
    let turn = plan_skill_turn("/weekly-summary", &catalog).unwrap();
    assert_eq!(turn.question, "Apply the skill \"weekly-summary\".");
}

#[test]
fn an_ordinary_message_is_not_a_skill_turn() {
    let catalog = catalog_of(vec![skill("weekly-summary", "Weekly recap.")]);
    assert_eq!(plan_skill_turn("weekly summary please", &catalog), None);
    assert_eq!(plan_skill_turn("/other thing", &catalog), None);
}

// ── overview ───────────────────────────────────────────────────────────

#[test]
fn overview_lists_skills_and_errors_even_when_the_feature_is_off() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::new(tmp.path().to_path_buf()).unwrap();
    let dir = tmp.path().join(SKILLS_DIR);
    write_skill(&dir, "alpha", &skill_md("alpha", "First.", "Body."));
    write_skill(&dir, "broken", "nope");
    db.set_preference(SKILLS_ENABLED_PREF, "false").unwrap();

    let o = overview(&db);
    assert!(!o.enabled);
    assert_eq!(o.dir.as_deref(), Some(dir.as_path()));
    assert_eq!(o.skills.len(), 1);
    assert_eq!(o.skills[0].name, "alpha");
    assert_eq!(o.skills[0].description, "First.");
    assert_eq!(o.errors.len(), 1);
    // Bodies stay out of the listing: it feeds Settings and the CLI.
    let json = serde_json::to_string(&o).unwrap();
    assert!(!json.contains("Body."), "{json}");
    assert!(json.contains("\"enabled\":false"), "{json}");
}

#[test]
fn ensure_skills_dir_creates_the_folder_once() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::new(tmp.path().to_path_buf()).unwrap();
    let dir = ensure_skills_dir(&db).unwrap();
    assert_eq!(dir, tmp.path().join(SKILLS_DIR));
    assert!(dir.is_dir());
    // Idempotent: a second call on an existing folder succeeds.
    assert_eq!(ensure_skills_dir(&db).unwrap(), dir);
}

#[test]
fn ensure_skills_dir_errors_without_a_data_dir() {
    let db = Database::new_for_testing().unwrap();
    assert!(ensure_skills_dir(&db).is_err());
}
