"""The backup-tables gate: which tables the migrations leave, and the lists."""

import importlib.util
import pathlib

SCRIPT = pathlib.Path(__file__).resolve().parent.parent / "check_backup_tables.py"
spec = importlib.util.spec_from_file_location("check_backup_tables", SCRIPT)
check_backup_tables = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check_backup_tables)

TABLES_RS = """
pub const BACKED_UP: &[BackedUp] = &[
    all("accounts"),
    BackedUp {
        table: "user_preferences",
        only: Some("key <> 'x'"),
    },
];

pub const NOT_BACKED_UP: &[(&str, Reason)] = &[
    ("emails", Reason::FromServer),
];
"""


def test_create_drop_and_rename_are_replayed_in_order():
    sql = [
        "CREATE TABLE IF NOT EXISTS emails (id TEXT);\nCREATE TABLE old_name (x);",
        "CREATE VIRTUAL TABLE IF NOT EXISTS emails_fts USING fts5(body);\n"
        "ALTER TABLE old_name RENAME TO new_name;\nDROP TABLE IF EXISTS emails;",
    ]

    assert check_backup_tables.tables_after(sql) == {"emails_fts", "new_name"}


def test_commented_out_statements_do_not_count():
    sql = ["-- CREATE TABLE ghost (x);\n/* DROP TABLE emails; */\nCREATE TABLE emails (id);"]

    assert check_backup_tables.tables_after(sql) == {"emails"}


def test_migrations_are_applied_by_version_number(tmp_path):
    for name in ["V010__b.sql", "V002__a.sql", "README.md"]:
        (tmp_path / name).write_text("")

    assert [p.name for p in check_backup_tables.migration_files(tmp_path)] == ["V002__a.sql", "V010__b.sql"]


def test_both_lists_are_read_from_tables_rs():
    backed_up, not_backed_up = check_backup_tables.listed_tables(TABLES_RS)

    assert backed_up == ["accounts", "user_preferences"]
    assert not_backed_up == ["emails"]


def test_a_new_table_nobody_classified_is_reported():
    found = check_backup_tables.problems({"accounts", "emails", "lens_notes"}, ["accounts"], ["emails"])

    assert found == ["not classified: lens_notes"]


def test_a_listed_table_a_migration_dropped_is_reported():
    found = check_backup_tables.problems({"accounts"}, ["accounts"], ["emails"])

    assert found == ["listed but no migration leaves them: emails"]


def test_a_table_listed_twice_is_reported():
    found = check_backup_tables.problems({"accounts"}, ["accounts"], ["accounts"])

    assert found == ["listed twice: accounts"]


def test_the_repository_lists_every_table():
    assert check_backup_tables.main() == 0
