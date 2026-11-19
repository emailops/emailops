"""Security invariants of .github/workflows/release.yml.

The signing jobs hold the Certum and GPG secrets and start only after an owner
approval, possibly hours after the build. They must run the commit that was
built, not whatever `inputs.tag_name` points at by then; no job that runs the
npm/cargo dependency tree may hold a write token; and a dry run of an arbitrary
ref must not seed the build caches a signed release later restores.
"""

import re
from pathlib import Path

import yaml

WORKFLOW = Path(__file__).resolve().parents[2] / ".github" / "workflows" / "release.yml"
PINNED_SHA = "${{ needs.resolve.outputs.sha }}"


def _workflow() -> dict:
    return yaml.safe_load(WORKFLOW.read_text())


def _jobs() -> dict:
    return _workflow()["jobs"]


def _contents_permission(job_name: str) -> str | None:
    """The `contents` scope the job's token gets: its own block, else the workflow's."""
    workflow = _workflow()
    permissions = workflow["jobs"][job_name].get("permissions", workflow.get("permissions", {}))
    return permissions.get("contents")


def _steps(uses_prefix: str) -> list[tuple[str, dict]]:
    return [
        (name, step)
        for name, job in _jobs().items()
        for step in job.get("steps", [])
        if step.get("uses", "").startswith(uses_prefix)
    ]


def _needs(job: dict) -> list[str]:
    needs = job.get("needs", [])
    return [needs] if isinstance(needs, str) else needs


def test_resolve_job_outputs_the_commit_with_a_read_only_token():
    assert _contents_permission("resolve") == "read"
    assert "sha" in _jobs()["resolve"]["outputs"]


def test_every_checkout_after_resolve_uses_the_resolved_commit():
    wrong = [
        (name, step.get("with", {}).get("ref"))
        for name, step in _steps("actions/checkout@")
        if name != "resolve" and step.get("with", {}).get("ref") != PINNED_SHA
    ]
    assert wrong == []


def test_jobs_that_check_out_the_resolved_commit_need_resolve_directly():
    # `needs.<job>.outputs` is empty unless that job is a direct dependency.
    jobs = _jobs()
    missing = sorted(
        {name for name, _ in _steps("actions/checkout@") if name != "resolve" and "resolve" not in _needs(jobs[name])}
    )
    assert missing == []


def test_no_checkout_persists_credentials():
    persisted = [name for name, step in _steps("actions/checkout@") if step.get("with", {}).get("persist-credentials") is not False]
    assert persisted == []


def test_workflow_token_defaults_to_read_only():
    # Without a workflow-level block, jobs that set none get the repository
    # default, which may be read-write.
    assert _workflow()["permissions"] == {"contents": "read"}


def test_only_publishing_jobs_get_a_write_token():
    writers = {name for name in _jobs() if _contents_permission(name) == "write"}
    assert writers == {"linux-sign", "windows-publish"}


def test_jobs_that_run_the_dependency_tree_cannot_write():
    builders = {
        name
        for name, job in _jobs().items()
        for step in job.get("steps", [])
        if "npm ci" in step.get("run", "") or "make build-" in step.get("run", "")
    }
    assert builders, "no job runs npm ci or make build-: the workflow layout changed"
    writers = {name for name in builders if _contents_permission(name) == "write"}
    assert writers == set()


def test_every_job_builds_with_one_pinned_rust_release():
    # A floating `stable` broke the release when Rust 1.100 changed the
    # OUT_DIR layout that llama-cpp-sys-2's build script walks up from.
    toolchains = {step.get("with", {}).get("toolchain") for _, step in _steps("dtolnay/rust-toolchain@")}
    assert len(toolchains) == 1
    assert re.fullmatch(r"1\.\d+\.\d+", toolchains.pop() or "")


def test_dry_runs_never_save_the_rust_cache():
    unscoped = [name for name, step in _steps("Swatinem/rust-cache@") if step.get("with", {}).get("save-if") != "${{ !inputs.dry_run }}"]
    assert unscoped == []
