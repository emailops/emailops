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
        files: Vec::new(),
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
            names: vec!["vendor-reply".into()],
            rest: "the quote from ACME".into()
        })
    );
    assert_eq!(
        plan_invocation("  /weekly-summary  ", &names),
        Some(Invocation {
            names: vec!["weekly-summary".into()],
            rest: String::new()
        })
    );
    assert_eq!(
        plan_invocation("/weekly-summary\nsolo clientes de Madrid", &names),
        Some(Invocation {
            names: vec!["weekly-summary".into()],
            rest: "solo clientes de Madrid".into()
        })
    );
}

#[test]
fn the_skill_name_is_matched_case_insensitively() {
    assert_eq!(
        plan_invocation("/Weekly-Summary", &["weekly-summary"]).map(|i| i.names),
        Some(vec!["weekly-summary".to_string()])
    );
}

#[test]
fn several_leading_skills_stack_until_the_first_other_token() {
    // Hermes-style stacking: `/a /b request`. Parsing stops at the first
    // token that is not a known skill, so a path in the request survives.
    let names = ["weekly-summary", "vendor-reply"];
    assert_eq!(
        plan_invocation("/vendor-reply /weekly-summary /tmp/report.pdf please", &names),
        Some(Invocation {
            names: vec!["vendor-reply".into(), "weekly-summary".into()],
            rest: "/tmp/report.pdf please".into()
        })
    );
    // A skill named twice is applied once.
    assert_eq!(
        plan_invocation("/vendor-reply /vendor-reply go", &names).map(|i| i.names),
        Some(vec!["vendor-reply".to_string()])
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
    // Experimental: off until the user switches it on.
    assert!(!skills_enabled(&db));
    db.set_preference(SKILLS_ENABLED_PREF, "true").unwrap();
    assert!(skills_enabled(&db));
}

#[test]
fn a_disabled_skill_leaves_the_catalog_but_stays_listed() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::new(tmp.path().to_path_buf()).unwrap();
    db.set_preference(SKILLS_ENABLED_PREF, "true").unwrap();
    let dir = tmp.path().join(SKILLS_DIR);
    write_skill(&dir, "alpha", &skill_md("alpha", "First.", "Body."));
    write_skill(&dir, "beta", &skill_md("beta", "Second.", "Body."));

    set_skill_enabled(&db, "alpha", false).unwrap();
    assert_eq!(catalog_for(&db).names(), vec!["beta"]);
    let listed: Vec<(String, bool)> = overview(&db).skills.into_iter().map(|s| (s.name, s.enabled)).collect();
    assert_eq!(listed, vec![("alpha".to_string(), false), ("beta".to_string(), true)]);

    set_skill_enabled(&db, "alpha", true).unwrap();
    assert_eq!(catalog_for(&db).names(), vec!["alpha", "beta"]);
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
    assert_eq!(turn.skills, vec!["vendor-reply".to_string()]);
    assert_eq!(turn.question, "the ACME quote");
    assert_eq!(turn.block, render_skill_block(&catalog.skills[0]));
}

#[test]
fn a_bare_invocation_asks_to_apply_the_skill() {
    // Retrieval, the planner and the conversation title all read the
    // question; an empty one would give them nothing to work with.
    let catalog = catalog_of(vec![skill("weekly-summary", "Weekly recap.")]);
    let turn = plan_skill_turn("/weekly-summary", &catalog).unwrap();
    // The description says what the user wants done; a made-up "Apply the
    // skill" sentence sent retrieval, routing and the title the wrong way.
    assert_eq!(turn.question, "Weekly recap.");
}

#[test]
fn a_stacked_turn_carries_every_block_in_order() {
    let catalog = catalog_of(vec![skill("a-skill", "A."), skill("b-skill", "B.")]);
    let turn = plan_skill_turn("/b-skill /a-skill", &catalog).unwrap();
    assert_eq!(turn.skills, vec!["b-skill".to_string(), "a-skill".to_string()]);
    assert_eq!(turn.question, "B. A.");
    let b = turn.block.find("<skill name=\"b-skill\">").unwrap();
    let a = turn.block.find("<skill name=\"a-skill\">").unwrap();
    assert!(b < a, "{}", turn.block);
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

// ── skills index (system prompt) ───────────────────────────────────────

#[test]
fn the_index_lists_skills_and_tells_the_model_to_load_first() {
    let index = render_skills_index(&[skill("vendor-support", "Find support contacts.")]).unwrap();
    assert!(index.contains("- vendor-support: Find support contacts."), "{index}");
    assert!(index.contains("load_skill"), "{index}");
    assert!(index.contains("FIRST"), "{index}");
    // It must never claim a load happened without the call.
    assert!(index.contains("Never say you loaded a skill"), "{index}");
}

#[test]
fn no_skills_means_no_index() {
    assert_eq!(render_skills_index(&[]), None);
}

// ── reference files (level 2) ──────────────────────────────────────────

#[test]
fn loading_lists_the_skills_text_files_but_not_skill_md_or_others() {
    let tmp = tempfile::tempdir().unwrap();
    write_skill(tmp.path(), "vendor-support", &skill_md("vendor-support", "d", "Body."));
    let dir = tmp.path().join("vendor-support");
    fs::create_dir_all(dir.join("references/deep/deeper")).unwrap();
    fs::write(dir.join("references/escalation.md"), "Escalate.").unwrap();
    fs::write(dir.join("templates.txt"), "Hi {name}").unwrap();
    fs::write(dir.join("logo.png"), [0u8, 1, 2]).unwrap();
    fs::write(dir.join("run.sh"), "rm -rf /").unwrap();
    fs::write(dir.join("references/deep/deeper/too-deep.md"), "x").unwrap();

    let catalog = load_catalog(tmp.path());
    assert_eq!(
        catalog.get("vendor-support").unwrap().files,
        vec!["references/escalation.md".to_string(), "templates.txt".to_string()]
    );
}

#[test]
fn the_skill_block_names_its_reference_files() {
    let mut s = skill("vendor-support", "d");
    s.files = vec!["references/escalation.md".into()];
    let block = render_skill_block(&s);
    assert!(block.contains("references/escalation.md"), "{block}");
    assert!(block.contains("load_skill"), "{block}");
    // No files, no mention.
    assert!(!render_skill_block(&skill("x", "d")).contains("Reference files"));
}

#[test]
fn the_skill_block_says_it_is_already_loaded() {
    // A `/name` turn carries the block in the user message; without this the
    // index's "call load_skill FIRST" made the model load it a second time.
    let block = render_skill_block(&skill("vendor-reply", "d"));
    assert!(block.contains("already here — do not load them again"), "{block}");
}

#[test]
fn the_already_loaded_note_still_lets_the_model_read_reference_files() {
    // Forbidding load_skill outright made small models skip the reference
    // files the body relies on.
    let mut s = skill("vendor-support", "d");
    s.files = vec!["references/escalation.md".into()];
    let block = render_skill_block(&s);
    assert!(!block.contains("do not call load_skill"), "{block}");
    assert!(block.contains("load_skill(name=\"vendor-support\", file="), "{block}");
}

#[test]
fn read_reference_serves_only_listed_files() {
    let tmp = tempfile::tempdir().unwrap();
    write_skill(tmp.path(), "vendor-support", &skill_md("vendor-support", "d", "Body."));
    let dir = tmp.path().join("vendor-support");
    fs::create_dir_all(dir.join("references")).unwrap();
    fs::write(dir.join("references/escalation.md"), "Escalate after 48h.").unwrap();
    fs::write(tmp.path().join("secret.md"), "not yours").unwrap();
    let catalog = load_catalog(tmp.path());
    let s = catalog.get("vendor-support").unwrap();

    assert_eq!(
        read_reference(s, "references/escalation.md").unwrap(),
        "Escalate after 48h."
    );
    // Leading `./` is tolerated.
    assert!(read_reference(s, "./references/escalation.md").is_ok());
    for bad in ["../secret.md", "/etc/passwd", "SKILL.md", "references/missing.md", ""] {
        assert!(read_reference(s, bad).is_err(), "{bad:?} must be refused");
    }
}

#[test]
fn a_long_reference_is_cut_with_a_visible_marker() {
    let tmp = tempfile::tempdir().unwrap();
    write_skill(tmp.path(), "big", &skill_md("big", "d", "Body."));
    fs::write(tmp.path().join("big/notes.md"), "y".repeat(MAX_BODY_CHARS + 50)).unwrap();
    let catalog = load_catalog(tmp.path());
    let text = read_reference(catalog.get("big").unwrap(), "notes.md").unwrap();
    assert!(text.ends_with("[truncated]"), "{}", &text[text.len() - 30..]);
    assert!(text.chars().count() <= MAX_BODY_CHARS + 20);
}

// ── render_planner_rule ────────────────────────────────────────────────

#[test]
fn the_planner_rule_is_empty_without_skills() {
    // An install without skills sends the planner the same prompt as before.
    assert_eq!(render_planner_rule(&[]), "");
}

#[test]
fn the_planner_rule_lists_the_skills_and_how_to_name_one() {
    let rule = render_planner_rule(&[
        skill("travel-brief", "Brief the user on a trip."),
        skill("a-skill", "First."),
    ]);
    assert!(
        rule.contains("- a-skill: First.\n- travel-brief: Brief the user on a trip."),
        "{rule}"
    );
    assert!(rule.contains(r#""skill": "<name>""#), "{rule}");
    // It rides alongside any verdict, a defer included.
    assert!(rule.contains(r#"{"defer": true, "skill": "<name>"}"#), "{rule}");
    // Naming a skill must not turn a defer into an invented filter.
    assert!(rule.contains("never changes the verdict"), "{rule}");
    // Descriptions only — never a body in the planner prompt.
    assert!(!rule.contains("Do the thing."), "{rule}");
}

// ── editing from the app ───────────────────────────────────────────────

fn db_in(tmp: &tempfile::TempDir) -> Database {
    Database::new(tmp.path().to_path_buf()).unwrap()
}

#[test]
fn create_writes_a_loadable_skill_and_refuses_a_second_one() {
    let tmp = tempfile::tempdir().unwrap();
    let db = db_in(&tmp);
    create_skill(&db, "weekly-digest").unwrap();
    let text = read_skill_source(&db, "weekly-digest").unwrap();
    assert!(
        parse_skill_md(&text, "weekly-digest").is_ok(),
        "the template must load: {text}"
    );
    let err = create_skill(&db, "weekly-digest").unwrap_err().to_string();
    assert!(err.contains("already exists"), "{err}");
}

#[test]
fn a_name_that_is_not_a_skill_name_never_reaches_the_disk() {
    let tmp = tempfile::tempdir().unwrap();
    let db = db_in(&tmp);
    for bad in ["", "../escape", "Upper", "a/b", "-x"] {
        assert!(create_skill(&db, bad).is_err(), "{bad:?}");
        assert!(read_skill_source(&db, bad).is_err(), "{bad:?}");
        assert!(save_skill_source(&db, bad, "x", "x").is_err(), "{bad:?}");
    }
    assert!(!tmp.path().join("escape").exists());
}

#[test]
fn save_validates_before_writing() {
    let tmp = tempfile::tempdir().unwrap();
    let db = db_in(&tmp);
    create_skill(&db, "alpha").unwrap();
    let before = read_skill_source(&db, "alpha").unwrap();

    let err = save_skill_source(&db, "alpha", "no frontmatter", &before)
        .unwrap_err()
        .to_string();
    assert!(err.contains("frontmatter"), "{err}");
    assert_eq!(
        read_skill_source(&db, "alpha").unwrap(),
        before,
        "an invalid save leaves the file alone"
    );

    let good = skill_md("alpha", "Updated.", "New steps.");
    assert_eq!(save_skill_source(&db, "alpha", &good, &before).unwrap(), "alpha");
    assert_eq!(read_skill_source(&db, "alpha").unwrap(), good);
}

#[test]
fn save_only_edits_an_existing_skill() {
    let tmp = tempfile::tempdir().unwrap();
    let db = db_in(&tmp);
    let err = save_skill_source(&db, "ghost", &skill_md("ghost", "D.", "B."), "")
        .unwrap_err()
        .to_string();
    assert!(err.contains("ghost"), "{err}");
    assert!(read_skill_source(&db, "ghost").is_err());
}

#[cfg(unix)]
#[test]
fn reference_files_never_follow_a_symlink_out_of_the_skill() {
    // A skill copied from elsewhere may ship links into the user's files;
    // listing them would hand those files to the model.
    let tmp = tempfile::tempdir().unwrap();
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("private.md"), "secret").unwrap();

    let dir = tmp.path().join(SKILLS_DIR);
    write_skill(&dir, "alpha", &skill_md("alpha", "First.", "Body."));
    let refs = dir.join("alpha/references");
    fs::create_dir_all(&refs).unwrap();
    fs::write(refs.join("real.md"), "fine").unwrap();
    std::os::unix::fs::symlink(outside.join("private.md"), refs.join("linked.md")).unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("alpha/linked-dir")).unwrap();

    let catalog = load_catalog(&dir);
    let skill = catalog.get("alpha").unwrap();
    assert_eq!(skill.files, vec!["references/real.md".to_string()]);
    assert!(read_reference(skill, "references/linked.md").is_err());
    assert!(read_reference(skill, "linked-dir/private.md").is_err());
}

#[test]
fn a_toggle_only_accepts_a_skill_name() {
    let tmp = tempfile::tempdir().unwrap();
    let db = db_in(&tmp);
    for bad in ["", "../x", "Upper", "a b"] {
        assert!(set_skill_enabled(&db, bad, false).is_err(), "{bad:?}");
    }
    assert_eq!(db.get_preference(SKILLS_DISABLED_PREF).unwrap(), None);
}

#[test]
fn concurrent_toggles_keep_every_change() {
    let tmp = tempfile::tempdir().unwrap();
    let db = std::sync::Arc::new(db_in(&tmp));
    let handles: Vec<_> = (0..16)
        .map(|i| {
            let db = db.clone();
            std::thread::spawn(move || set_skill_enabled(&db, &format!("s{i:02}"), false).unwrap())
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let stored: Vec<String> = serde_json::from_str(&db.get_preference(SKILLS_DISABLED_PREF).unwrap().unwrap()).unwrap();
    assert_eq!(stored.len(), 16, "{stored:?}");
}

#[test]
fn save_refuses_to_overwrite_an_edit_made_outside_the_app() {
    let tmp = tempfile::tempdir().unwrap();
    let db = db_in(&tmp);
    create_skill(&db, "alpha").unwrap();
    let opened = read_skill_source(&db, "alpha").unwrap();
    let theirs = skill_md("alpha", "Edited in another editor.", "Their steps.");
    fs::write(tmp.path().join(SKILLS_DIR).join("alpha").join(SKILL_FILE), &theirs).unwrap();

    let err = save_skill_source(&db, "alpha", &skill_md("alpha", "Mine.", "My steps."), &opened)
        .unwrap_err()
        .to_string();
    assert!(err.contains("changed on disk"), "{err}");
    assert_eq!(read_skill_source(&db, "alpha").unwrap(), theirs);
}

#[test]
fn saving_a_different_name_renames_the_skill_and_keeps_its_switch() {
    let tmp = tempfile::tempdir().unwrap();
    let db = db_in(&tmp);
    create_skill(&db, "weekly-report").unwrap();
    set_skill_enabled(&db, "weekly-report", false).unwrap();
    let base = read_skill_source(&db, "weekly-report").unwrap();
    let pasted = skill_md("weekly-email-summary", "Weekly recap.", "Steps.");

    assert_eq!(
        save_skill_source(&db, "weekly-report", &pasted, &base).unwrap(),
        "weekly-email-summary"
    );
    assert_eq!(read_skill_source(&db, "weekly-email-summary").unwrap(), pasted);
    assert!(!tmp.path().join(SKILLS_DIR).join("weekly-report").exists());
    let listed: Vec<(String, bool)> = overview(&db).skills.into_iter().map(|s| (s.name, s.enabled)).collect();
    assert_eq!(listed, vec![("weekly-email-summary".to_string(), false)]);
}

#[test]
fn a_rename_onto_an_existing_skill_changes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let db = db_in(&tmp);
    create_skill(&db, "alpha").unwrap();
    create_skill(&db, "beta").unwrap();
    let base = read_skill_source(&db, "alpha").unwrap();
    let err = save_skill_source(&db, "alpha", &skill_md("beta", "D.", "B."), &base)
        .unwrap_err()
        .to_string();
    assert!(err.contains("beta") && err.contains("already exists"), "{err}");
    assert_eq!(read_skill_source(&db, "alpha").unwrap(), base);
}

#[test]
fn an_invocation_of_a_switched_off_skill_is_recognised() {
    let disabled: std::collections::BTreeSet<String> = ["weekly-summary".to_string()].into();
    assert_eq!(
        invoked_disabled_skill("/weekly-summary last week", &disabled),
        Some("weekly-summary")
    );
    assert_eq!(invoked_disabled_skill("/other thing", &disabled), None);
    assert_eq!(invoked_disabled_skill("weekly-summary", &disabled), None);
}

#[test]
fn the_question_of_a_skill_turn_drops_the_invocation() {
    // Research estimates are keyed on the question the turn will ask.
    let catalog = catalog_of(vec![skill("weekly-summary", "Weekly recap.")]);
    assert_eq!(question_of("/weekly-summary acme", &catalog), "acme");
    assert_eq!(question_of("/weekly-summary", &catalog), "Weekly recap.");
    assert_eq!(question_of("plain question", &catalog), "plain question");
}
