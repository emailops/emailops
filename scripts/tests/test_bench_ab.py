"""scripts/bench_ab.py: interleaved prefill-speed A/B between emailops-cli builds."""

import json
import os
import pathlib
import shutil
import stat
import subprocess
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
from bench_ab import bucket_medians, prefill_rates, verdict  # noqa: E402

SCRIPTS = pathlib.Path(__file__).resolve().parents[1]


def envelope(*calls):
    """A `chat --fresh --json --trace` envelope with one turn per call."""
    turns = [
        {"trace": {"llmCalls": [dict({"failed": False, "cachedPromptTokens": 0}, **c)]}}
        for c in calls
    ]
    return {"ok": True, "data": {"turns": turns}, "error": None}


def test_prefill_rate_counts_only_the_uncached_tokens():
    env = envelope({"promptTokens": 9000, "cachedPromptTokens": 8000, "prefillMs": 500})

    assert prefill_rates(env) == [(1000, 2000.0)]


def test_prefill_rates_skip_small_failed_and_untimed_calls():
    env = envelope(
        {"promptTokens": 2600, "cachedPromptTokens": 2580, "prefillMs": 5},
        {"promptTokens": 4000, "prefillMs": 1000, "failed": True},
        {"promptTokens": 4000, "prefillMs": None},
        {"promptTokens": 1024, "prefillMs": 512},
    )

    assert prefill_rates(env) == [(1024, 2000.0)]


def test_bucket_medians_split_long_from_short_prefills():
    rates = [(8000, 1500.0), (9000, 1700.0), (900, 1000.0), (1000, 1200.0), (950, 1100.0)]

    assert bucket_medians(rates) == {"long": 1600.0, "short": 1100.0}


def test_verdict_better_when_every_candidate_beats_its_neighbours_by_the_bar():
    result, deltas = verdict(controls=[100, 100, 100, 100], candidates=[110, 108, 112])

    assert result == "better"
    assert [round(d, 2) for d in deltas] == [0.10, 0.08, 0.12]


def test_verdict_worse_when_every_candidate_trails_its_neighbours():
    result, _ = verdict(controls=[100, 100, 100, 100], candidates=[90, 92, 88])

    assert result == "worse"


def test_verdict_noise_when_one_candidate_loses_to_its_neighbours():
    result, _ = verdict(controls=[100, 100, 100, 100], candidates=[110, 97, 112])

    assert result == "noise"


def test_verdict_noise_when_the_median_gain_is_under_the_bar():
    result, _ = verdict(controls=[100, 100, 100, 100], candidates=[102, 101, 102])

    assert result == "noise"


def test_verdict_compares_each_candidate_with_the_controls_around_it():
    # Drift: the machine slows down over the session. Against the first
    # control alone the candidates would all look slower.
    result, deltas = verdict(controls=[100, 90, 80, 70], candidates=[100, 90, 80])

    assert result == "better"
    assert [round(d, 3) for d in deltas] == [0.053, 0.059, 0.067]


# --- end to end, against fake emailops-cli builds ----------------------------


def _executable(path, body):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(body)
    path.chmod(path.stat().st_mode | stat.S_IEXEC)


def _fake_cli(path, log, name, env):
    path.parent.mkdir(parents=True, exist_ok=True)
    (path.parent / f"{name}.json").write_text(json.dumps(env))
    _executable(path, f'#!/bin/sh\necho {name} >> "{log}"\ncat "{path.parent}/{name}.json"\n')


def _run_ab(tmp_path, control_env, candidate_env):
    repo = tmp_path / "repo"
    (repo / "scripts").mkdir(parents=True)
    for name in ("bench_ab.py", "bench_preflight.sh"):
        shutil.copy(SCRIPTS / name, repo / "scripts" / name)
    demo = tmp_path / "demo"
    demo.mkdir()
    (demo / "emailops.db").write_text("")
    (tmp_path / "models").mkdir()
    log = tmp_path / "runs.log"
    _fake_cli(tmp_path / "cli" / "ctl", log, "ctl", control_env)
    _fake_cli(tmp_path / "cli" / "cand", log, "cand", candidate_env)
    _executable(tmp_path / "bin" / "pmset", "#!/bin/sh\necho \"Now drawing from 'AC Power'\"\n")
    _executable(tmp_path / "bin" / "pgrep", "#!/bin/sh\nexit 1\n")
    env = dict(
        os.environ,
        PATH=f"{tmp_path / 'bin'}{os.pathsep}{os.environ['PATH']}",
        EMAILOPS_DEMO_DIR=str(demo),
        MODELS_DIR=str(tmp_path / "models"),
    )
    proc = subprocess.run(
        [
            sys.executable, str(repo / "scripts" / "bench_ab.py"),
            "--control", f"ctl={tmp_path / 'cli' / 'ctl'}",
            "--candidate", f"cand={tmp_path / 'cli' / 'cand'}",
            "--model", "some-model",
        ],
        capture_output=True, text=True, env=env,
    )
    runs = log.read_text().split() if log.exists() else []
    return proc, runs


SLOW = envelope({"promptTokens": 9000, "prefillMs": 6000}, {"promptTokens": 1000, "prefillMs": 1000})
FAST = envelope({"promptTokens": 9000, "prefillMs": 3000}, {"promptTokens": 1000, "prefillMs": 500})


def test_three_interleaved_candidate_runs_settle_a_clear_difference(tmp_path):
    proc, runs = _run_ab(tmp_path, SLOW, FAST)

    assert proc.returncode == 0, proc.stderr
    assert runs == ["ctl", "cand", "ctl", "cand", "ctl", "cand", "ctl"]


def test_arms_within_noise_get_six_candidate_runs(tmp_path):
    proc, runs = _run_ab(tmp_path, SLOW, SLOW)

    assert proc.returncode == 0, proc.stderr
    assert runs.count("cand") == 6
    assert runs[-1] == "ctl"


def test_a_failed_run_stops_the_ab(tmp_path):
    failed = {"ok": False, "data": None, "error": {"code": "not_found", "message": "Not found"}}

    proc, runs = _run_ab(tmp_path, failed, FAST)

    assert proc.returncode != 0
    assert runs == ["ctl"]
    assert "not_found" in proc.stderr
