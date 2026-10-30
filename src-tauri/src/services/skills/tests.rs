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
    let err = parse_skill_md("# Just markdown", "x").unwrap_err().to_string();
    assert!(err.contains("frontmatter"), "{err}");
}

#[test]
fn rejects_unterminated_frontmatter() {
    let err = parse_skill_md("---\nname: x\ndescription: y\n", "x")
        .unwrap_err()
        .to_string();
    assert!(err.contains("frontmatter"), "{err}");
}

#[test]
fn rejects_missing_name_or_description() {
    let no_desc = "---\nname: x\n---\nbody";
    assert!(parse_skill_md(no_desc, "x")
        .unwrap_err()
        .to_string()
        .contains("description"));
    let no_name = "---\ndescription: y\n---\nbody";
    assert!(parse_skill_md(no_name, "x").unwrap_err().to_string().contains("name"));
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
    let err = parse_skill_md(&skill_md("weekly", "d", "b"), "monthly")
        .unwrap_err()
        .to_string();
    assert!(err.contains("monthly"), "{err}");
}

#[test]
fn rejects_an_empty_body() {
    let err = parse_skill_md(&skill_md("x", "d", "  \n"), "x")
        .unwrap_err()
        .to_string();
    assert!(err.contains("instructions"), "{err}");
}

#[test]
fn rejects_an_overlong_description_or_body_instead_of_cutting_it() {
    let long_desc = "d".repeat(MAX_DESCRIPTION_CHARS + 1);
    assert!(parse_skill_md(&skill_md("x", &long_desc, "b"), "x").is_err());
    let long_body = "b".repeat(MAX_BODY_CHARS + 1);
    let err = parse_skill_md(&skill_md("x", "d", &long_body), "x")
        .unwrap_err()
        .to_string();
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

// ── render_skill_block ─────────────────────────────────────────────────

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
fn a_bare_invocation_asks_what_the_skill_is_for() {
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

// ── reference files ────────────────────────────────────────────────────

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
fn the_skill_block_tells_the_model_to_follow_it() {
    let block = render_skill_block(&skill("vendor-reply", "d"));
    assert!(
        block.contains("Its instructions are already loaded — do not load them again. Follow them:"),
        "{block}"
    );
    assert!(!block.contains("load_skill"), "{block}");
}

#[test]
fn only_a_skill_with_files_points_at_load_skill() {
    let mut s = skill("vendor-support", "d");
    s.files = vec!["references/escalation.md".into()];
    let block = render_skill_block(&s);
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

// ── error codes (translated in the UI) ─────────────────────────────────

#[test]
fn parse_problems_carry_a_code_for_the_ui_to_translate() {
    let long_body = "x".repeat(MAX_BODY_CHARS + 1);
    for (text, folder, code) in [
        ("# Just markdown".to_string(), "x", "skill_no_frontmatter"),
        (
            "---\nname: x\ndescription: y\n".to_string(),
            "x",
            "skill_unclosed_frontmatter",
        ),
        (skill_md("weekly", "d", "b"), "monthly", "skill_name_mismatch"),
        ("---\nname: x\n---\nbody".to_string(), "x", "skill_no_description"),
        (skill_md("x", "d", &long_body), "x", "skill_body_too_long"),
    ] {
        assert_eq!(parse_skill_md(&text, folder).unwrap_err().code(), code, "{text:.40}");
    }
    let mismatch = parse_skill_md(&skill_md("weekly", "d", "b"), "monthly").unwrap_err();
    assert_eq!(mismatch.params().get("name").map(String::as_str), Some("weekly"));
    assert_eq!(mismatch.params().get("folder").map(String::as_str), Some("monthly"));
}

#[test]
fn a_skill_problem_crosses_the_command_boundary_with_its_code() {
    let err = crate::models::error::AppError::from(SkillProblem::AlreadyExists("beta".into()));
    assert_eq!(err.code(), "skill_already_exists");
    assert_eq!(err.params().get("name").map(String::as_str), Some("beta"));
    assert!(err.to_string().contains("beta"), "{err}");
}

#[test]
fn editing_errors_are_coded() {
    let tmp = tempfile::tempdir().unwrap();
    let db = db_in(&tmp);
    create_skill(&db, "alpha").unwrap();
    assert_eq!(create_skill(&db, "alpha").unwrap_err().code(), "skill_already_exists");
    assert_eq!(
        set_skill_enabled(&db, "Bad Name", false).unwrap_err().code(),
        "skill_invalid_name"
    );
    let base = read_skill_source(&db, "alpha").unwrap();
    fs::write(
        tmp.path().join(SKILLS_DIR).join("alpha").join(SKILL_FILE),
        "edited elsewhere",
    )
    .unwrap();
    let err = save_skill_source(&db, "alpha", &skill_md("alpha", "D.", "B."), &base).unwrap_err();
    assert_eq!(err.code(), "skill_changed_on_disk");
}

#[test]
fn load_errors_carry_the_same_code_and_params() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join(SKILLS_DIR);
    write_skill(&dir, "broken", "no frontmatter here");
    let catalog = load_catalog(&dir);
    assert_eq!(catalog.errors[0].code, "skill_no_frontmatter");
    assert!(catalog.errors[0].message.contains("frontmatter"));
}

// ── delete ─────────────────────────────────────────────────────────────

#[test]
fn delete_moves_the_skill_aside_and_forgets_its_switch() {
    let tmp = tempfile::tempdir().unwrap();
    let db = db_in(&tmp);
    create_skill(&db, "alpha").unwrap();
    set_skill_enabled(&db, "alpha", false).unwrap();
    delete_skill(&db, "alpha").unwrap();

    assert!(overview(&db).skills.is_empty());
    assert!(disabled_skills(&db).is_empty());
    // Recoverable: the folder is kept under .deleted, which the catalog skips.
    let kept = tmp
        .path()
        .join(SKILLS_DIR)
        .join(DELETED_DIR)
        .join("alpha")
        .join(SKILL_FILE);
    assert!(kept.is_file(), "{}", kept.display());
    assert!(overview(&db).errors.is_empty());
}

#[test]
fn deleting_the_same_name_twice_keeps_both_copies() {
    let tmp = tempfile::tempdir().unwrap();
    let db = db_in(&tmp);
    for _ in 0..2 {
        create_skill(&db, "alpha").unwrap();
        delete_skill(&db, "alpha").unwrap();
    }
    let deleted = tmp.path().join(SKILLS_DIR).join(DELETED_DIR);
    assert!(deleted.join("alpha").is_dir());
    assert!(deleted.join("alpha-2").is_dir());
}

// ── bundled skills ─────────────────────────────────────────────────────

const BUNDLED_NAME: &str = "inbox-triage";

fn db_with_feature_on(tmp: &tempfile::TempDir) -> Database {
    let db = db_in(tmp);
    db.set_preference(SKILLS_ENABLED_PREF, "true").unwrap();
    db
}

fn bundled_folder() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("bundled-skills")
}

#[test]
fn every_bundled_skill_loads() {
    let catalog = load_bundled();
    assert_eq!(catalog.errors, Vec::new());
    assert_eq!(catalog.names(), vec!["follow-up-check", "inbox-triage"]);
}

#[test]
fn the_bundled_list_and_the_folder_agree() {
    let mut on_disk: Vec<String> = fs::read_dir(bundled_folder())
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    on_disk.sort();
    assert_eq!(on_disk, load_bundled().names());
    for name in &on_disk {
        let file = bundled_folder().join(name).join(SKILL_FILE);
        assert_eq!(
            bundled_source(name),
            Some(fs::read_to_string(file).unwrap().as_str()),
            "{name}"
        );
    }
}

#[test]
fn a_bundled_skill_holds_only_its_skill_md() {
    // Reference files are read from disk, so one in a bundled skill would be
    // promised to the model and never reach it.
    for skill in load_bundled().skills {
        let entries: Vec<String> = fs::read_dir(bundled_folder().join(&skill.name))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, vec![SKILL_FILE.to_string()], "{}", skill.name);
    }
}

#[test]
fn the_bundled_folder_records_its_source_commit() {
    let source = fs::read_to_string(bundled_folder().join("SOURCE")).unwrap();
    let commit = source.split_whitespace().last().unwrap_or_default();
    assert_eq!(commit.len(), 40, "{source}");
    assert!(commit.chars().all(|c| c.is_ascii_hexdigit()), "{source}");
}

#[test]
fn bundled_skills_are_listed_off_while_the_feature_is_off() {
    let tmp = tempfile::tempdir().unwrap();
    let o = overview(&db_in(&tmp));
    assert!(!o.enabled);
    let listed: Vec<(&str, bool)> = o.bundled.iter().map(|b| (b.name.as_str(), b.enabled)).collect();
    assert_eq!(listed, vec![("follow-up-check", false), ("inbox-triage", false)]);
    assert!(!o.bundled[0].description.is_empty());
    assert!(o.skills.is_empty());
}

#[test]
fn with_every_bundled_skill_off_the_catalog_is_the_users_own() {
    let tmp = tempfile::tempdir().unwrap();
    let db = db_with_feature_on(&tmp);
    let dir = tmp.path().join(SKILLS_DIR);
    assert_eq!(catalog_for(&db), SkillCatalog::default());

    write_skill(&dir, "alpha", &skill_md("alpha", "First.", "Body."));
    assert_eq!(catalog_for(&db), load_catalog(&dir));
}

#[test]
fn a_switched_on_bundled_skill_works_like_any_other() {
    let tmp = tempfile::tempdir().unwrap();
    let db = db_with_feature_on(&tmp);
    write_skill(
        &tmp.path().join(SKILLS_DIR),
        "zeta",
        &skill_md("zeta", "Last.", "Body."),
    );
    set_bundled_skill_enabled(&db, BUNDLED_NAME, true).unwrap();

    let catalog = catalog_for(&db);
    assert_eq!(catalog.names(), vec![BUNDLED_NAME, "zeta"]);
    let turn = plan_skill_turn("/inbox-triage what first?", &catalog).unwrap();
    assert_eq!(turn.skills, vec![BUNDLED_NAME]);
    assert!(turn.block.contains("list_open_threads"), "{}", turn.block);

    set_bundled_skill_enabled(&db, BUNDLED_NAME, false).unwrap();
    assert_eq!(catalog_for(&db).names(), vec!["zeta"]);
}

#[test]
fn a_switched_on_bundled_skill_needs_the_feature() {
    let tmp = tempfile::tempdir().unwrap();
    let db = db_in(&tmp);
    set_bundled_skill_enabled(&db, BUNDLED_NAME, true).unwrap();
    assert_eq!(catalog_for(&db), SkillCatalog::default());
    let listed = overview(&db);
    assert!(listed.bundled.iter().any(|b| b.name == BUNDLED_NAME && b.enabled));
}

#[test]
fn only_a_bundled_skill_can_be_switched_on_as_one() {
    let tmp = tempfile::tempdir().unwrap();
    let db = db_with_feature_on(&tmp);
    let err = set_bundled_skill_enabled(&db, "not-bundled", true).unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)), "{err}");
    assert_eq!(bundled_skills_on(&db), Default::default());
}

#[test]
fn concurrent_bundled_toggles_keep_every_change() {
    let tmp = tempfile::tempdir().unwrap();
    let db = std::sync::Arc::new(db_with_feature_on(&tmp));
    let threads: Vec<_> = ["follow-up-check", "inbox-triage"]
        .into_iter()
        .map(|name| {
            let db = db.clone();
            std::thread::spawn(move || set_bundled_skill_enabled(&db, name, true).unwrap())
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(bundled_skills_on(&db).len(), 2);
}

#[test]
fn nothing_bundled_is_written_to_the_skills_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let db = db_with_feature_on(&tmp);
    set_bundled_skill_enabled(&db, BUNDLED_NAME, true).unwrap();
    catalog_for(&db);
    overview(&db);
    read_bundled_source(BUNDLED_NAME).unwrap();
    assert!(!tmp.path().join(SKILLS_DIR).exists());
}

#[test]
fn a_user_skill_with_a_bundled_name_wins() {
    let tmp = tempfile::tempdir().unwrap();
    let db = db_with_feature_on(&tmp);
    set_bundled_skill_enabled(&db, BUNDLED_NAME, true).unwrap();
    write_skill(
        &tmp.path().join(SKILLS_DIR),
        BUNDLED_NAME,
        &skill_md(BUNDLED_NAME, "Mine.", "My own steps."),
    );

    let catalog = catalog_for(&db);
    assert_eq!(catalog.names(), vec![BUNDLED_NAME]);
    assert_eq!(catalog.get(BUNDLED_NAME).unwrap().body, "My own steps.");
    let listed = overview(&db);
    let bundled = listed.bundled.iter().find(|b| b.name == BUNDLED_NAME).unwrap();
    assert!(bundled.shadowed);
}

#[test]
fn a_user_skill_keeps_a_bundled_one_inactive_while_it_is_off_or_broken() {
    let tmp = tempfile::tempdir().unwrap();
    let db = db_with_feature_on(&tmp);
    let dir = tmp.path().join(SKILLS_DIR);
    set_bundled_skill_enabled(&db, BUNDLED_NAME, true).unwrap();

    write_skill(&dir, BUNDLED_NAME, &skill_md(BUNDLED_NAME, "Mine.", "Body."));
    set_skill_enabled(&db, BUNDLED_NAME, false).unwrap();
    assert_eq!(catalog_for(&db).names(), Vec::<&str>::new());

    set_skill_enabled(&db, BUNDLED_NAME, true).unwrap();
    write_skill(&dir, BUNDLED_NAME, "no frontmatter");
    assert_eq!(catalog_for(&db).names(), Vec::<&str>::new());
    assert_eq!(catalog_for(&db).errors.len(), 1);
}

#[test]
fn the_bundled_text_is_read_raw_and_only_for_bundled_names() {
    let text = read_bundled_source(BUNDLED_NAME).unwrap();
    assert!(text.starts_with("---\nname: inbox-triage\n"), "{text}");
    let err = read_bundled_source("not-bundled").unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)), "{err}");
}

#[test]
fn a_copy_is_a_normal_user_skill() {
    let tmp = tempfile::tempdir().unwrap();
    let db = db_with_feature_on(&tmp);
    set_bundled_skill_enabled(&db, BUNDLED_NAME, true).unwrap();
    copy_bundled_skill(&db, BUNDLED_NAME).unwrap();

    let dir = tmp.path().join(SKILLS_DIR);
    assert_eq!(load_catalog(&dir).names(), vec![BUNDLED_NAME]);
    assert_eq!(
        read_skill_source(&db, BUNDLED_NAME).unwrap(),
        read_bundled_source(BUNDLED_NAME).unwrap()
    );
    assert!(overview(&db).skills.iter().any(|s| s.name == BUNDLED_NAME && s.enabled));
    assert!(catalog_for(&db)
        .get(BUNDLED_NAME)
        .is_some_and(|s| s.path.ends_with("inbox-triage/SKILL.md")));
}

#[test]
fn a_copy_of_a_switched_off_bundled_skill_stays_off() {
    let tmp = tempfile::tempdir().unwrap();
    let db = db_with_feature_on(&tmp);
    copy_bundled_skill(&db, BUNDLED_NAME).unwrap();
    assert_eq!(catalog_for(&db), {
        let mut empty = load_catalog(&tmp.path().join(SKILLS_DIR));
        empty.skills.clear();
        empty
    });
    assert!(overview(&db)
        .skills
        .iter()
        .any(|s| s.name == BUNDLED_NAME && !s.enabled));
}

#[test]
fn a_copy_refuses_a_name_that_exists_and_leaves_that_folder_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let db = db_with_feature_on(&tmp);
    let dir = tmp.path().join(SKILLS_DIR);
    let mine = skill_md(BUNDLED_NAME, "Mine.", "My own steps.");
    write_skill(&dir, BUNDLED_NAME, &mine);
    fs::write(dir.join(BUNDLED_NAME).join("notes.md"), "keep me").unwrap();

    let err = copy_bundled_skill(&db, BUNDLED_NAME).unwrap_err().to_string();

    assert!(err.contains("already exists"), "{err}");
    assert_eq!(
        fs::read_to_string(dir.join(BUNDLED_NAME).join(SKILL_FILE)).unwrap(),
        mine
    );
    assert_eq!(
        fs::read_to_string(dir.join(BUNDLED_NAME).join("notes.md")).unwrap(),
        "keep me"
    );
    assert!(disabled_skills(&db).is_empty());
}

#[test]
fn only_a_bundled_skill_can_be_copied() {
    let tmp = tempfile::tempdir().unwrap();
    let db = db_with_feature_on(&tmp);
    let err = copy_bundled_skill(&db, "not-bundled").unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)), "{err}");
    assert!(!tmp.path().join(SKILLS_DIR).exists());
}

#[test]
fn an_invocation_of_a_switched_off_bundled_skill_is_recognised() {
    let tmp = tempfile::tempdir().unwrap();
    let db = db_with_feature_on(&tmp);
    let off = switched_off_skills(&db);
    assert_eq!(
        invoked_disabled_skill("/inbox-triage what first?", &off),
        Some(BUNDLED_NAME)
    );
    set_bundled_skill_enabled(&db, BUNDLED_NAME, true).unwrap();
    let off = switched_off_skills(&db);
    assert_eq!(invoked_disabled_skill("/inbox-triage what first?", &off), None);
}

#[test]
fn deleting_a_missing_or_badly_named_skill_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let db = db_in(&tmp);
    assert_eq!(delete_skill(&db, "ghost").unwrap_err().code(), "not_found");
    assert_eq!(delete_skill(&db, "../x").unwrap_err().code(), "skill_invalid_name");
}
