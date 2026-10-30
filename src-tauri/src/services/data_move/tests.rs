use std::path::{Path, PathBuf};

use super::*;
use crate::db::Database;
use crate::util::data_location::{self, DataDirSource};

const GB: u64 = 1_000_000_000;

fn room(free_bytes: u64) -> TargetFacts {
    TargetFacts {
        exists: false,
        is_empty: true,
        writable: true,
        free_bytes,
    }
}

fn plan(current: &str, target: &str, facts: TargetFacts) -> std::result::Result<(), MoveProblem> {
    plan_move(Path::new(current), Path::new(target), &facts, 2 * GB)
}

#[test]
fn a_new_folder_on_a_disk_with_room_is_accepted() {
    assert_eq!(plan("/data", "/Volumes/Ext/EmailOps", room(10 * GB)), Ok(()));
}

#[test]
fn an_existing_empty_folder_is_accepted() {
    let facts = TargetFacts {
        exists: true,
        ..room(10 * GB)
    };
    assert_eq!(plan("/data", "/Volumes/Ext/EmailOps", facts), Ok(()));
}

#[test]
fn the_env_override_cannot_be_moved_from_the_app() {
    assert_eq!(ensure_movable(DataDirSource::Env), Err(MoveProblem::EnvOverride));
}

#[test]
fn a_chosen_or_default_folder_can_be_moved() {
    assert_eq!(ensure_movable(DataDirSource::Pointer), Ok(()));
    assert_eq!(ensure_movable(DataDirSource::Default), Ok(()));
}

#[test]
fn the_current_folder_is_refused() {
    assert_eq!(plan("/data", "/data", room(10 * GB)), Err(MoveProblem::SameFolder));
}

#[test]
fn a_folder_inside_the_current_one_is_refused() {
    assert_eq!(
        plan("/data", "/data/models/EmailOps", room(10 * GB)),
        Err(MoveProblem::InsideCurrent)
    );
}

#[test]
fn a_sibling_whose_name_extends_the_current_one_is_not_inside_it() {
    assert_eq!(plan("/data", "/data-ext/EmailOps", room(10 * GB)), Ok(()));
}

#[test]
fn a_folder_with_files_is_refused() {
    let facts = TargetFacts {
        exists: true,
        is_empty: false,
        ..room(10 * GB)
    };
    assert_eq!(
        plan("/data", "/Volumes/Ext/EmailOps", facts),
        Err(MoveProblem::NotEmpty {
            path: "/Volumes/Ext/EmailOps".into()
        })
    );
}

#[test]
fn a_read_only_place_is_refused() {
    let facts = TargetFacts {
        writable: false,
        ..room(10 * GB)
    };
    assert_eq!(
        plan("/data", "/Volumes/Ext/EmailOps", facts),
        Err(MoveProblem::NotWritable {
            path: "/Volumes/Ext/EmailOps".into()
        })
    );
}

#[test]
fn a_disk_without_room_for_the_data_is_refused() {
    assert_eq!(
        plan("/data", "/Volumes/Ext/EmailOps", room(GB)),
        Err(MoveProblem::NotEnoughSpace {
            needed_bytes: 2 * GB,
            available_bytes: GB
        })
    );
}

#[test]
fn a_picked_folder_gets_its_own_emailops_subfolder() {
    assert_eq!(
        target_for_picked(Path::new("/Volumes/Ext")),
        PathBuf::from("/Volumes/Ext/EmailOps")
    );
}

#[test]
fn facts_treat_a_finder_metadata_file_as_empty() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".DS_Store"), b"x").unwrap();
    let facts = target_facts(dir.path());
    assert!(facts.exists && facts.is_empty && facts.writable);
}

#[test]
fn facts_see_a_folder_with_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.txt"), b"x").unwrap();
    assert!(!target_facts(dir.path()).is_empty);
}

#[test]
fn facts_for_a_folder_still_to_be_created() {
    let dir = tempfile::tempdir().unwrap();
    let facts = target_facts(&dir.path().join("EmailOps"));
    assert!(!facts.exists && facts.is_empty && facts.writable && facts.free_bytes > 0);
}

/// A data folder shaped like a real one: a database with a row, a model,
/// an attachment, a backup and the instance lock.
fn seeded_data_dir(root: &Path) -> Database {
    let db = Database::new(root.to_path_buf()).unwrap();
    db.set_preference("ui_language", "es").unwrap();
    std::fs::create_dir_all(root.join("models/embed")).unwrap();
    std::fs::write(root.join("models/embed/model.gguf"), vec![7u8; 4096]).unwrap();
    std::fs::create_dir_all(root.join("attachments/acc-1")).unwrap();
    std::fs::write(root.join("attachments/acc-1/invoice.pdf"), b"%PDF-1.4").unwrap();
    std::fs::create_dir_all(root.join("backups")).unwrap();
    std::fs::write(root.join("backups/emailops-20260101_000000.db"), b"old").unwrap();
    std::fs::write(root.join("emailops.lock"), b"123").unwrap();
    db
}

struct Dirs {
    _scratch: tempfile::TempDir,
    default: PathBuf,
    current: PathBuf,
    target: PathBuf,
}

fn dirs() -> Dirs {
    let scratch = tempfile::tempdir().unwrap();
    let default = scratch.path().join("com.emailops.app");
    let target = scratch.path().join("external").join("EmailOps");
    Dirs {
        current: default.clone(),
        default,
        target,
        _scratch: scratch,
    }
}

#[test]
fn moving_copies_everything_and_points_the_next_launch_there() {
    let d = dirs();
    let db = seeded_data_dir(&d.current);

    move_data_dir(&db, &d.current, &d.target, &d.default, &mut |_, _| {}).unwrap();

    assert_eq!(data_location::read_pointer(&d.default).unwrap(), Some(d.target.clone()));
    let moved = Database::new(d.target.clone()).unwrap();
    assert_eq!(moved.get_preference("ui_language").unwrap().as_deref(), Some("es"));
    assert_eq!(
        std::fs::read(d.target.join("models/embed/model.gguf")).unwrap(),
        vec![7u8; 4096]
    );
    assert_eq!(
        std::fs::read(d.target.join("attachments/acc-1/invoice.pdf")).unwrap(),
        b"%PDF-1.4"
    );
    assert!(d.target.join("backups/emailops-20260101_000000.db").is_file());
}

#[test]
fn moving_keeps_the_users_skills_and_every_switch() {
    use crate::services::skills;

    let d = dirs();
    let db = seeded_data_dir(&d.current);
    let mine = d.current.join("skills/weekly-digest");
    std::fs::create_dir_all(mine.join("references")).unwrap();
    std::fs::write(mine.join("SKILL.md"), "my skill").unwrap();
    std::fs::write(mine.join("references/tone.md"), "my tone").unwrap();
    let deleted = d.current.join("skills/.deleted/old-skill");
    std::fs::create_dir_all(&deleted).unwrap();
    std::fs::write(deleted.join("SKILL.md"), "deleted skill").unwrap();
    db.set_preference(skills::SKILLS_ENABLED_PREF, "true").unwrap();
    skills::set_skill_enabled(&db, "weekly-digest", false).unwrap();
    skills::set_bundled_skill_enabled(&db, "inbox-triage", true).unwrap();

    move_data_dir(&db, &d.current, &d.target, &d.default, &mut |_, _| {}).unwrap();

    let moved = Database::new(d.target.clone()).unwrap();
    assert_eq!(
        std::fs::read(d.target.join("skills/weekly-digest/SKILL.md")).unwrap(),
        b"my skill"
    );
    assert_eq!(
        std::fs::read(d.target.join("skills/weekly-digest/references/tone.md")).unwrap(),
        b"my tone"
    );
    assert_eq!(
        std::fs::read(d.target.join("skills/.deleted/old-skill/SKILL.md")).unwrap(),
        b"deleted skill"
    );
    assert!(skills::skills_enabled(&moved));
    assert!(skills::disabled_skills(&moved).contains("weekly-digest"));
    assert!(skills::bundled_skills_on(&moved).contains("inbox-triage"));
}

#[test]
fn moving_does_not_carry_the_instance_lock() {
    let d = dirs();
    let db = seeded_data_dir(&d.current);
    move_data_dir(&db, &d.current, &d.target, &d.default, &mut |_, _| {}).unwrap();
    assert!(!d.target.join("emailops.lock").exists());
}

#[test]
fn moving_leaves_the_old_folder_untouched() {
    let d = dirs();
    let db = seeded_data_dir(&d.current);
    move_data_dir(&db, &d.current, &d.target, &d.default, &mut |_, _| {}).unwrap();
    assert!(d.current.join("emailops.db").is_file());
    assert!(d.current.join("models/embed/model.gguf").is_file());
}

#[test]
fn the_moved_database_remembers_where_the_old_copy_is() {
    let d = dirs();
    let db = seeded_data_dir(&d.current);
    move_data_dir(&db, &d.current, &d.target, &d.default, &mut |_, _| {}).unwrap();
    let moved = Database::new(d.target.clone()).unwrap();
    assert_eq!(
        moved.get_preference(PREVIOUS_LOCATION_PREF).unwrap(),
        Some(d.current.to_string_lossy().into_owned())
    );
    assert_eq!(db.get_preference(PREVIOUS_LOCATION_PREF).unwrap(), None);
}

#[test]
fn moving_back_to_the_default_removes_the_pointer() {
    let d = dirs();
    std::fs::create_dir_all(d.default.parent().unwrap()).unwrap();
    let elsewhere = d.target.clone();
    let db = seeded_data_dir(&elsewhere);
    data_location::write_pointer(&d.default, &elsewhere).unwrap();

    move_data_dir(&db, &elsewhere, &d.default, &d.default, &mut |_, _| {}).unwrap();

    assert_eq!(data_location::read_pointer(&d.default).unwrap(), None);
    assert!(d.default.join("emailops.db").is_file());
}

#[test]
fn progress_reaches_the_total() {
    let d = dirs();
    let db = seeded_data_dir(&d.current);
    let mut last = (0, 0);
    move_data_dir(&db, &d.current, &d.target, &d.default, &mut |copied, total| {
        last = (copied, total)
    })
    .unwrap();
    assert!(last.1 > 0);
    assert_eq!(last.0, last.1);
}

#[test]
fn a_refused_move_writes_nothing() {
    let d = dirs();
    let db = seeded_data_dir(&d.current);
    std::fs::create_dir_all(&d.target).unwrap();
    std::fs::write(d.target.join("someone-elses.txt"), b"x").unwrap();

    let err = move_data_dir(&db, &d.current, &d.target, &d.default, &mut |_, _| {}).unwrap_err();

    assert!(matches!(err, AppError::DataMove(MoveProblem::NotEmpty { .. })));
    assert_eq!(data_location::read_pointer(&d.default).unwrap(), None);
    assert!(!d.target.join("emailops.db").exists());
}

/// A model adopted with "Use existing file…": a link into the models folder
/// to a GGUF the user keeps elsewhere.
#[cfg(unix)]
fn link_a_model(data_dir: &Path, own_file: &Path) -> PathBuf {
    let link = data_dir.join("models/chat/linked.gguf");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(own_file, &link).unwrap();
    link
}

#[cfg(unix)]
#[test]
fn moving_keeps_a_linked_model_a_link() {
    let d = dirs();
    let db = seeded_data_dir(&d.current);
    let own_file = d._scratch.path().join("Downloads/linked.gguf");
    std::fs::create_dir_all(own_file.parent().unwrap()).unwrap();
    std::fs::write(&own_file, vec![3u8; 2048]).unwrap();
    link_a_model(&d.current, &own_file);

    move_data_dir(&db, &d.current, &d.target, &d.default, &mut |_, _| {}).unwrap();

    let moved = d.target.join("models/chat/linked.gguf");
    assert!(std::fs::symlink_metadata(&moved).unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read_link(&moved).unwrap(), own_file);
    assert_eq!(std::fs::read(&own_file).unwrap(), vec![3u8; 2048]);
}

#[cfg(unix)]
#[test]
fn a_linked_model_does_not_count_toward_the_bytes_needed() {
    let d = dirs();
    let _db = seeded_data_dir(&d.current);
    let without = check_move(&d.current, &d.target).unwrap();
    let own_file = d._scratch.path().join("Downloads/linked.gguf");
    std::fs::create_dir_all(own_file.parent().unwrap()).unwrap();
    std::fs::write(&own_file, vec![3u8; 2048]).unwrap();
    link_a_model(&d.current, &own_file);

    assert_eq!(check_move(&d.current, &d.target).unwrap(), without);
}

#[cfg(unix)]
#[test]
fn a_dangling_link_does_not_stop_the_move() {
    let d = dirs();
    let db = seeded_data_dir(&d.current);
    let gone = d._scratch.path().join("Downloads/deleted.gguf");
    link_a_model(&d.current, &gone);

    move_data_dir(&db, &d.current, &d.target, &d.default, &mut |_, _| {}).unwrap();

    let moved = d.target.join("models/chat/linked.gguf");
    assert_eq!(std::fs::read_link(&moved).unwrap(), gone);
}

#[cfg(unix)]
#[test]
fn a_relative_link_still_reaches_its_file_from_the_new_folder() {
    let d = dirs();
    let db = seeded_data_dir(&d.current);
    let own_file = d.current.join("own.gguf");
    std::fs::write(&own_file, vec![5u8; 64]).unwrap();
    let link = d.current.join("models/chat/linked.gguf");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink("../../own.gguf", &link).unwrap();

    move_data_dir(&db, &d.current, &d.target, &d.default, &mut |_, _| {}).unwrap();

    let moved = d.target.join("models/chat/linked.gguf");
    assert_eq!(
        std::fs::read_link(&moved).unwrap(),
        d.current.join("models/chat/../../own.gguf")
    );
}

#[cfg(unix)]
#[test]
fn the_new_folder_is_readable_only_by_the_user() {
    use std::os::unix::fs::PermissionsExt;
    let d = dirs();
    let db = seeded_data_dir(&d.current);

    move_data_dir(&db, &d.current, &d.target, &d.default, &mut |_, _| {}).unwrap();

    let mode = std::fs::metadata(&d.target).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
}

#[cfg(unix)]
#[test]
fn an_existing_empty_folder_is_locked_down_too() {
    use std::os::unix::fs::PermissionsExt;
    let d = dirs();
    let db = seeded_data_dir(&d.current);
    std::fs::create_dir_all(&d.target).unwrap();
    std::fs::set_permissions(&d.target, std::fs::Permissions::from_mode(0o755)).unwrap();

    move_data_dir(&db, &d.current, &d.target, &d.default, &mut |_, _| {}).unwrap();

    let mode = std::fs::metadata(&d.target).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
}

#[test]
fn checking_a_move_reports_the_bytes_it_needs() {
    let d = dirs();
    let _db = seeded_data_dir(&d.current);
    let needed = check_move(&d.current, &d.target).unwrap();
    assert!(needed >= 4096 + 8, "model and attachment at least: {needed}");
}

#[test]
fn checking_a_move_into_the_current_folder_is_refused() {
    let d = dirs();
    let _db = seeded_data_dir(&d.current);
    let err = check_move(&d.current, &d.current.join("models").join("EmailOps")).unwrap_err();
    assert!(matches!(err, AppError::DataMove(MoveProblem::InsideCurrent)));
}

#[test]
fn sizes_in_messages_round_up_to_whole_units_like_the_ui() {
    assert_eq!(size_label(3 * 1024 * 1024 * 1024), "3 GB");
    assert_eq!(size_label(3 * 1024 * 1024 * 1024 + 1), "4 GB");
    assert_eq!(size_label(1), "1 MB");
}
