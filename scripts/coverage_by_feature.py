#!/usr/bin/env python3
"""Rank coverage by feature and by file from the reports scripts/coverage.sh writes.

Rust: reads the lcov export and counts PRODUCTION lines only. Stable rustc has no
`#[coverage(off)]`, so inline `#[cfg(test)] mod tests { … }` blocks and files that
are only declared under `#[cfg(test)]` (test_helpers.rs, tests.rs, …) are cut out
here; otherwise every file's percentage would be inflated by its own tests.
TS: reads vitest's json-summary (tests are already excluded by the vitest config).

Features come from .claude/skills/verify-emailops/features.json (`rust` = module
prefixes, `vitest` = path prefixes; the longest matching prefix wins, as in
verify_all.py). Unmatched files land in "Transversal".

Usage: coverage_by_feature.py <reports/coverage dir>   → writes by-feature.md there.
"""

import json
import re
import sys
from collections import defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SRC = REPO / "src-tauri" / "src"
MANIFEST = json.loads((REPO / ".claude/skills/verify-emailops/features.json").read_text())

CFG_TEST = re.compile(r"^\s*#\[cfg\(test\)\]\s*$")
ATTR = re.compile(r"^\s*#\[")
MOD_DECL = re.compile(r"^\s*(?:pub(?:\([a-z]+\))?\s+)?mod\s+([a-z_0-9]+)\s*;")
MOD_OPEN = re.compile(r"^\s*(?:pub(?:\([a-z]+\))?\s+)?mod\s+[a-z_0-9]+\s*\{")


def block_end(lines, start):
    """Index of the line closing the `{` opened on `lines[start]` (string/char/comment aware)."""
    depth = 0
    for i in range(start, len(lines)):
        s = lines[i]
        j = 0
        while j < len(s):
            c = s[j]
            if s.startswith("//", j):
                break
            if c == "r" and re.match(r'r#*"', s[j:]):
                hashes = re.match(r'r(#*)"', s[j:]).group(1)
                close = '"' + hashes
                k = s.find(close, j + 2 + len(hashes))
                # Multi-line raw strings are rare in tests; treat the rest of the line as string.
                j = len(s) if k < 0 else k + len(close)
                continue
            if c == '"':
                j += 1
                while j < len(s) and s[j] != '"':
                    j += 2 if s[j] == "\\" else 1
                j += 1
                continue
            m = re.match(r"'(\\.|[^\\'])'", s[j:])
            if m:
                j += len(m.group(0))
                continue
            if c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
                if depth == 0:
                    return i
            j += 1
    return len(lines) - 1


def test_ranges(path):
    """(1-based inclusive line ranges of inline test modules, set of test-only child files)."""
    lines = path.read_text(errors="replace").splitlines()
    ranges, child_files = [], set()
    for i, line in enumerate(lines):
        if not CFG_TEST.match(line):
            continue
        k = i + 1
        while k < len(lines) and ATTR.match(lines[k]):
            k += 1
        if k >= len(lines):
            continue
        decl = MOD_DECL.match(lines[k])
        if decl:
            base = path.parent if path.name in ("mod.rs", "lib.rs", "main.rs") else path.with_suffix("")
            for cand in (base / f"{decl.group(1)}.rs", base / decl.group(1) / "mod.rs"):
                child_files.add(cand.resolve())
        elif MOD_OPEN.match(lines[k]):
            ranges.append((i + 1, block_end(lines, k) + 1))
    return ranges, child_files


def module_of(rel):
    """src-tauri/src/services/emails/mod.rs → services::emails."""
    parts = list(Path(rel).with_suffix("").parts)
    if parts[-1] in ("mod", "lib", "main"):
        parts = parts[:-1]
    return "::".join(parts)


def feature_for(key, kind):
    best, best_len = "Transversal", -1
    for f in MANIFEST["features"]:
        for p in f.get(kind, []):
            if kind == "rust":
                hit = key == p or key.startswith(p + "::")
            else:
                hit = key.startswith(p)
            if hit and len(p) > best_len:
                best, best_len = f["name"], len(p)
    return best


def rust_regions(export, cut_by_file, test_files):
    """file → (regions, covered) over production code, from the full llvm-cov JSON export.

    Generic functions appear once per instantiation, so a region is keyed by its
    span and counts as covered when any instantiation ran it.
    """
    spans = defaultdict(dict)
    for fn in json.loads(export.read_text())["data"][0]["functions"]:
        names = fn["filenames"]
        for ls, cs, le, ce, count, file_id, _exp, kind in fn["regions"]:
            if kind != 0:  # code regions only (skip expansion/skipped/gap regions)
                continue
            f = Path(names[file_id])
            if f.resolve() in test_files or any(a <= ls <= b for a, b in cut_by_file.get(f, [])):
                continue
            key = (ls, cs, le, ce)
            spans[f][key] = spans[f].get(key, False) or count > 0
    return {f: (len(r), sum(r.values())) for f, r in spans.items()}


def rust_rows(lcov):
    per_file = defaultdict(dict)
    current = None
    for line in lcov.read_text().splitlines():
        if line.startswith("SF:"):
            current = Path(line[3:])
        elif line.startswith("DA:") and current is not None:
            n, hits = line[3:].split(",")[:2]
            per_file[current][int(n)] = max(per_file[current].get(int(n), 0), int(hits))
    test_files, ranges = set(), {}
    for f in per_file:
        if f.is_file():
            r, children = test_ranges(f)
            ranges[f] = r
            test_files |= children
    export = lcov.parent / "coverage.json"
    regions = rust_regions(export, ranges, test_files) if export.is_file() else {}
    rows = []
    for f, das in per_file.items():
        if f.resolve() in test_files or not str(f).startswith(str(SRC)):
            continue
        cut = ranges.get(f, [])
        prod = {n: h for n, h in das.items() if not any(a <= n <= b for a, b in cut)}
        if not prod:
            continue
        missed = sorted(n for n, h in prod.items() if h == 0)
        rel = f.relative_to(SRC)
        rows.append({
            "file": f"src-tauri/src/{rel}",
            "feature": feature_for(module_of(rel), "rust"),
            "lines": len(prod),
            "covered": len(prod) - len(missed),
            "missed_lines": missed,
            "branches": regions.get(f, (0, 0))[0],
            "branches_covered": regions.get(f, (0, 0))[1],
        })
    return rows


def ts_rows(summary):
    data = json.loads(summary.read_text())
    rows = []
    for f, s in data.items():
        if f == "total":
            continue
        rel = str(Path(f).resolve().relative_to(REPO))
        rows.append({
            "file": rel,
            "feature": feature_for(rel, "vitest"),
            "lines": s["lines"]["total"],
            "covered": s["lines"]["covered"],
            "branches": s["branches"]["total"],
            "branches_covered": s["branches"]["covered"],
        })
    return rows


def pct(c, t):
    return f"{100 * c / t:.1f}%" if t else "n/a"


def spans(missed):
    """Largest contiguous runs of uncovered lines (gaps of <=2 covered/blank lines merged)."""
    out, start, prev = [], None, None
    for n in missed:
        if start is None:
            start = prev = n
        elif n - prev <= 3:
            prev = n
        else:
            out.append((start, prev))
            start = prev = n
    if start is not None:
        out.append((start, prev))
    return out


def feature_table(rows, title, branch_label=None):
    agg = defaultdict(lambda: [0, 0, 0, 0, 0])
    for r in rows:
        a = agg[r["feature"]]
        a[0] += r["lines"]
        a[1] += r["covered"]
        a[2] += r.get("branches", 0)
        a[3] += r.get("branches_covered", 0)
        a[4] += 1
    head = "| Feature | Files | Lines | Covered | Line % | Missed |" + (f" {branch_label} % |" if branch_label else "")
    sep = "|---|---:|---:|---:|---:|---:|" + ("---:|" if branch_label else "")
    out = [f"## {title} by feature", "", head, sep]
    total = [0, 0, 0, 0, 0]
    for name, a in sorted(agg.items(), key=lambda kv: kv[1][1] / max(kv[1][0], 1)):
        total = [x + y for x, y in zip(total, a)]
        extra = f" {pct(a[3], a[2])} |" if branch_label else ""
        out.append(f"| {name} | {a[4]} | {a[0]} | {a[1]} | {pct(a[1], a[0])} | {a[0] - a[1]} |{extra}")
    extra = f" {pct(total[3], total[2])} |" if branch_label else ""
    out.append(f"| **Total** | {total[4]} | {total[0]} | {total[1]} | **{pct(total[1], total[0])}** | {total[0] - total[1]} |{extra}")
    return out + [""]


def file_table(rows, title, branch_label, limit=60):
    out = [f"## {title} files, most uncovered lines first", "",
           f"| File | Feature | Lines | Line % | Missed | {branch_label} % |", "|---|---|---:|---:|---:|---:|"]
    for r in sorted(rows, key=lambda r: r["covered"] - r["lines"])[:limit]:
        out.append(f"| {r['file']} | {r['feature']} | {r['lines']} | {pct(r['covered'], r['lines'])} | "
                   f"{r['lines'] - r['covered']} | {pct(r.get('branches_covered', 0), r.get('branches', 0))} |")
    return out + [""]


def main():
    base = Path(sys.argv[1])
    md = ["# Coverage by feature", ""]
    lcov = base / "rust" / "lcov.info"
    if lcov.is_file():
        rows = rust_rows(lcov)
        md += ["Rust counts exclude inline `#[cfg(test)]` modules and test-only files. "
               "Region % is the stable-toolchain proxy for branch coverage.", ""]
        md += feature_table(rows, "Rust", "Region")
        md += file_table(rows, "Rust", "Region")
        low = [r for r in rows if r["lines"] >= 20 and r["covered"] / r["lines"] < 0.4]
        md += ["## Rust files under 40% line coverage (>=20 production lines)", ""]
        md += [f"- {r['file']} — {pct(r['covered'], r['lines'])} of {r['lines']}" for r in sorted(low, key=lambda r: r["covered"] / r["lines"])]
        md += ["", "## Rust: 25 largest uncovered spans", "", "| File | Lines | Size |", "|---|---|---:|"]
        all_spans = [(b - a + 1, r["file"], a, b) for r in rows for a, b in spans(r["missed_lines"])]
        for size, f, a, b in sorted(all_spans, reverse=True)[:25]:
            md.append(f"| {f} | {a}-{b} | {size} |")
        md.append("")
    summary = base / "ts" / "coverage-summary.json"
    if summary.is_file():
        rows = ts_rows(summary)
        md += feature_table(rows, "TypeScript", "Branch")
        md += file_table(rows, "TypeScript", "Branch")
    (base / "by-feature.md").write_text("\n".join(md))
    print(f"by-feature: {base / 'by-feature.md'}")


if __name__ == "__main__":
    main()
