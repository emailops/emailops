"""The disk-clean planner: which build output under /Volumes/Build can go."""

import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
from disk_clean import RELEASE_TRIPLES, BuildDir, plan  # noqa: E402

DAY = 86400.0
NOW = 100 * DAY


def build_dir(name, days_idle=None, profiles=("debug",)):
    last_write = None if days_idle is None else NOW - days_idle * DAY
    return BuildDir(name=name, path=f"/Volumes/Build/emailops/{name}", last_write=last_write, profiles=list(profiles))


def run(dirs, worktrees=("emailopsv2",), main="emailopsv2", release=False):
    return [(a.kind, a.path) for a in plan(dirs, set(worktrees), main, NOW, idle_days=3, release=release)]


def test_build_dir_without_a_checkout_is_removed_whole():
    assert run([build_dir("gone-worktree", days_idle=0)]) == [("orphan", "/Volumes/Build/emailops/gone-worktree")]


def test_idle_worktree_target_is_emptied():
    actions = run([build_dir("old-feature", days_idle=4)], worktrees=("emailopsv2", "old-feature"))
    assert actions == [("idle", "/Volumes/Build/emailops/old-feature/target")]


def test_recently_built_worktree_is_kept():
    assert run([build_dir("busy", days_idle=1)], worktrees=("emailopsv2", "busy")) == []


def test_main_checkout_is_never_emptied_for_being_idle():
    assert run([build_dir("emailopsv2", days_idle=30)]) == []


def test_empty_target_needs_no_action():
    assert run([build_dir("fresh", days_idle=None, profiles=())], worktrees=("emailopsv2", "fresh")) == []


def test_release_builds_are_kept_by_default():
    dirs = [build_dir("emailopsv2", days_idle=0, profiles=("debug", *RELEASE_TRIPLES))]
    assert run(dirs) == []


def test_release_flag_removes_only_the_triples_present():
    dirs = [build_dir("emailopsv2", days_idle=0, profiles=("debug", "release", "aarch64-apple-darwin"))]
    assert run(dirs, release=True) == [("release", "/Volumes/Build/emailops/emailopsv2/target/aarch64-apple-darwin")]


def test_release_flag_does_not_repeat_a_target_already_emptied():
    dirs = [build_dir("old", days_idle=10, profiles=("debug", "x86_64-apple-darwin"))]
    actions = run(dirs, worktrees=("emailopsv2", "old"), release=True)
    assert actions == [("idle", "/Volumes/Build/emailops/old/target")]
