"""Split the published docs into claim-bearing blocks and read their markers.

Shared by check-docs-claims.py (every block must be marked) and
check_docs_run.py (every marked claim must be verified). Keeping the parser in
one place matters: if the guard and the runner disagreed about where a block
starts, a paragraph could be "covered" for one and invisible to the other.

A block is anything a reader could take as a statement about the app:

- a paragraph                      marker on its own line just above it
- a list item                      marker at the end of the item's last line
- a table, a fenced code block, a blockquote   marker on its own line above

A paragraph ending in ':' that is immediately followed by a table or a code
block introduces it ("Make it executable and run it:"), so the two are one
block with one marker. Followed by a list, it stays its own block: its text
may carry claims before the colon, and a lead-in that genuinely says nothing
is recorded as such in the catalog rather than silently exempted.

Markers are HTML comments. Hugo's goldmark renders them as
`<!-- raw HTML omitted -->`, so they never reach a reader.
"""

import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parent.parent
SITE = ROOT / "docs/site"
LANGS = ("en", "es", "fr", "de")
CATALOG = SITE / "claims.toml"

LINE_MARKER = re.compile(r"^<!--\s*claim:([a-z0-9-]+)\s*-->$")
TRAILING_MARKER = re.compile(r"\s*<!--\s*claim:([a-z0-9-]+)\s*-->\s*$")
LIST_ITEM = re.compile(r"^(?:[-*]|\d+\.)\s")
HEADING = re.compile(r"^#{1,6}\s")


class Block:
    def __init__(self, kind, start, lines, heading, claim, section=""):
        self.kind = kind          # paragraph | item | table | code | quote
        self.start = start        # 1-based line number of the first content line
        self.lines = lines        # content lines, markers stripped
        self.heading = heading    # nearest heading above, "" before the first
        self.claim = claim        # marker id, or None when unmarked
        self.section = section    # the H2 above, even when an H3 is nearer

    @property
    def text(self):
        """The block as a reader sees it: markup kept, markers gone."""
        return "\n".join(self.lines).strip()

    def __repr__(self):
        return f"<{self.kind}@{self.start} {self.claim or '-'}>"


def pages(lang="en"):
    """Published pages in sidebar order (_index first, then by weight)."""
    found = []
    for p in (SITE / lang).glob("*.md"):
        fm = front_matter(p)
        weight = -1 if p.name == "_index.md" else int(fm.get("weight", 999))
        found.append((weight, p))
    return [p for _, p in sorted(found)]


def front_matter(path):
    text = path.read_text(encoding="utf-8")
    if not text.startswith("---\n"):
        return {}
    out = {}
    for line in text.split("---", 2)[1].splitlines():
        if ":" in line:
            k, v = line.split(":", 1)
            out[k.strip()] = v.strip().strip("'\"")
    return out


def blocks(path):
    """Parse one page into blocks, in reading order."""
    raw = path.read_text(encoding="utf-8").splitlines()
    i = 0
    if raw and raw[0] == "---":
        i = raw.index("---", 1) + 1

    out = []
    heading = ""
    section = ""
    pending = None  # claim id from a marker line, waiting for its block

    def take(kind, start, lines, claim):
        out.append(Block(kind, start + 1, lines, heading, claim, section))

    while i < len(raw):
        line = raw[i]
        stripped = line.strip()

        if not stripped:
            i += 1
            continue
        m = LINE_MARKER.match(stripped)
        if m:
            pending = m.group(1)
            i += 1
            continue
        if HEADING.match(line):
            heading = re.sub(r"\s*\{#[a-z0-9-]+\}\s*$", "", line.lstrip("#").strip())
            if line.startswith("## "):
                section = heading
            pending = None
            i += 1
            continue

        start = i
        if stripped.startswith("```"):
            j = i + 1
            while j < len(raw) and not raw[j].strip().startswith("```"):
                j += 1
            take("code", start, raw[i : j + 1], pending)
            i = j + 1
        elif stripped.startswith("|"):
            j = i
            while j < len(raw) and raw[j].strip().startswith("|"):
                j += 1
            take("table", start, raw[i:j], pending)
            i = j
        elif stripped.startswith(">"):
            j = i
            while j < len(raw) and raw[j].strip().startswith(">"):
                j += 1
            take("quote", start, raw[i:j], pending)
            i = j
        elif LIST_ITEM.match(line):
            j = i + 1
            # continuation: indented, non-blank, not a new item or marker
            while j < len(raw) and raw[j].startswith(("  ", "\t")) and raw[j].strip():
                j += 1
            lines = raw[i:j]
            # The marker closes the item's text: at the end of its last line,
            # where any emphasis must already be closed. On the first line it
            # can land inside a **bold span** that wraps onto the next line.
            claim = None
            for n, text in enumerate(lines):
                m = TRAILING_MARKER.search(text)
                if m:
                    claim = m.group(1)
                    lines = lines[:n] + [text[: m.start()]] + lines[n + 1 :]
                    break
            # "2. Make it executable and run it:" owns the command it introduces.
            k = j
            while k < len(raw) and not raw[k].strip():
                k += 1
            if lines[-1].rstrip().endswith(":") and k < len(raw) and raw[k].strip().startswith("```"):
                e = k + 1
                while e < len(raw) and not raw[e].strip().startswith("```"):
                    e += 1
                lines = lines + [""] + raw[k : e + 1]
                j = e + 1
            take("item", start, lines, claim)
            i = j
        else:
            j = i
            while (
                j < len(raw)
                and raw[j].strip()
                and not HEADING.match(raw[j])
                and not LIST_ITEM.match(raw[j])
                and not LINE_MARKER.match(raw[j].strip())
                and not raw[j].strip().startswith(("```", "|", ">"))
            ):
                j += 1
            lines = raw[i:j]
            claim = pending
            # A lead-in ending in ':' absorbs the table or code it introduces.
            k = j
            while k < len(raw) and not raw[k].strip():
                k += 1
            nxt = raw[k].strip() if k < len(raw) else ""
            if lines[-1].rstrip().endswith(":") and nxt.startswith(("```", "|")):
                if nxt.startswith("```"):
                    e = k + 1
                    while e < len(raw) and not raw[e].strip().startswith("```"):
                        e += 1
                    lines = lines + [""] + raw[k : e + 1]
                    j = e + 1
                else:
                    e = k
                    while e < len(raw) and raw[e].strip().startswith("|"):
                        e += 1
                    lines = lines + [""] + raw[k:e]
                    j = e
            take("paragraph", start, lines, claim)
            i = j
        pending = None
    return out


def load_catalog():
    import tomllib

    with CATALOG.open("rb") as f:
        return tomllib.load(f)
