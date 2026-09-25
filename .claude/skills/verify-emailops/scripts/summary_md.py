#!/usr/bin/env python3
"""Compact Markdown summary of one `make verify` run, committed under docs/verification/.

    summary_md.py <run>/results.json <out_dir>

Writes <out_dir>/<run stamp>-<short commit>.md and prints its path. The full HTML report
stays local (src-tauri/reports/ is gitignored); this file is the durable record: commit,
totals, per-feature counts, what changed since the previous full run, and what fails.
Private runs (real mailbox) are refused: their content must never reach git.
"""
import collections, datetime, json, pathlib, sys

STATUSES = ("ok", "fail", "skip", "info")


def previous_run(run_dir):
    """Latest `*-full` sibling older than run_dir that recorded its commit, or None."""
    run_dir = pathlib.Path(run_dir)
    older = sorted(p for p in run_dir.parent.glob("[0-9]*-full")
                   if p.name < run_dir.name and (p / "results.json").exists()
                   and json.loads((p / "results.json").read_text()).get("meta", {}).get("commit"))
    return older[-1] if older else None


def short_commit(data):
    return data["meta"]["commit"].split()[0][:7]


def file_name(data):
    stamp = pathlib.Path(data["meta"]["run_dir"]).name.rsplit("-", 1)[0]
    return f"{stamp}-{short_commit(data)}.md"


def _key(r):
    return (r["feature"], r["type"], r["name"])


def _line(r):
    category = (r.get("evidence") or {}).get("category")
    detail = (r.get("detail") or "").strip().splitlines()
    return (f"- [{r['type']}] {r['feature']} :: {r['name']}"
            + (f" · {category}" if category else "")
            + (f" — {detail[0][:160]}" if detail else ""))


def _counts_row(records):
    c = collections.Counter(r["status"] for r in records)
    return "| " + " | ".join(str(c[s]) for s in STATUSES) + " |"


def summarize(data, prev):
    meta, records = data["meta"], data["records"]
    if "verify-private" in meta["run_dir"]:
        raise ValueError("refusing to summarise a private run: it carries real mailbox content")
    started = datetime.datetime.fromisoformat(meta["started"])
    minutes = (datetime.datetime.fromisoformat(meta["finished"]) - started).total_seconds() / 60
    subject = meta["commit"].split(" ", 1)[1] if " " in meta["commit"] else ""
    evals = meta.get("evals") or {}
    out = [
        f"# Verification {started:%d/%m/%Y %H:%M} — `{short_commit(data)}`",
        "",
        f"- Commit: `{short_commit(data)}` {subject}",
        f"- Branch: {meta.get('branch', '?')}",
        f"- Tier: {meta['tier']} · {minutes:.1f} min",
        f"- Eval model: {evals.get('model', '?')} · judge: {evals.get('judge_model', '?')}",
        f"- Uncommitted at run time: {', '.join(d.strip() for d in meta.get('dirty') or []) or 'none'}",
        "",
        "## Totals",
        "",
        "| " + " | ".join(STATUSES) + " |",
        "|" + "---|" * len(STATUSES),
        _counts_row(records),
        "",
        "## By feature",
        "",
        "| feature | " + " | ".join(STATUSES) + " |",
        "|---|" + "---|" * len(STATUSES),
    ]
    by_feature = collections.defaultdict(list)
    for r in records:
        by_feature[r["feature"]].append(r)
    out += [f"| {f} " + _counts_row(rs) for f, rs in by_feature.items()]
    out.append("")
    if prev is None:
        out += ["## Since the previous run", "", "No previous full run to compare with.", ""]
    else:
        before = {_key(r): r["status"] for r in prev["records"]}
        now_passing = [r for r in records if r["status"] == "ok" and before.get(_key(r)) == "fail"]
        now_failing = [r for r in records if r["status"] == "fail" and before.get(_key(r)) != "fail"]
        prev_meta = prev["meta"]
        out += [f"## Since `{pathlib.Path(prev_meta['run_dir']).name}` (`{short_commit(prev)}`)", ""]
        for title, rs in (("Now passing", now_passing), ("Now failing", now_failing)):
            out += [f"### {title} ({len(rs)})", ""] + ([_line(r) for r in rs] or ["- none"]) + [""]
    failing = [r for r in records if r["status"] == "fail"]
    out += [f"## Failing ({len(failing)})", ""] + ([_line(r) for r in failing] or ["- none"]) + [""]
    return "\n".join(out)


def main():
    results, out_dir = pathlib.Path(sys.argv[1]).resolve(), pathlib.Path(sys.argv[2])
    data = json.loads(results.read_text())
    prev_dir = previous_run(results.parent)
    prev = json.loads((prev_dir / "results.json").read_text()) if prev_dir else None
    out_dir.mkdir(parents=True, exist_ok=True)
    path = out_dir / file_name(data)
    path.write_text(summarize(data, prev))
    print(path)


if __name__ == "__main__":
    main()
