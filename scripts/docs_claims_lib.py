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
# Brackets a region rewritten from code (make docs-gen). Layout only: the
# parser reads it as a blank line, so a lead-in still owns the table inside.
GENERATED_MARKER = re.compile(r"^<!--\s*/?generated:[a-z0-9-]+\s*-->$")


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


def blocks(path, with_headings=False):
    """Parse one page into blocks, in reading order.

    with_headings also emits the headings, as Block("heading") with a
    `level`, for renderers that lay the page out; the claim guard leaves it
    off because a heading states nothing and carries no marker."""
    raw = ["" if GENERATED_MARKER.match(l.strip()) else l for l in path.read_text(encoding="utf-8").splitlines()]
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
            if with_headings:
                h = Block("heading", i + 1, [heading], heading, None, section)
                h.level = len(line) - len(line.lstrip("#"))
                out.append(h)
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


# ── fragments: the unit a check covers and the report colours ───────────────
# A block states several things; a check that proves one sentence must not
# make the whole paragraph read as verified. So a block splits into fragments —
# a sentence, a table row, a code block — and every check says which ones it
# covers, by quoting the text it verifies (see locate()).

class Fragment:
    def __init__(self, kind, text, prefix=""):
        self.kind = kind      # text | header | row | code
        self.text = text      # the markdown source, as written
        self.prefix = prefix  # the list bullet in front of an item's first fragment

    def __repr__(self):
        return f"<{self.kind} {self.text[:40]!r}>"


ABBREVIATIONS = ("e.g", "i.e", "etc", "vs", "cf", "approx")
# Sentence end: . ! or ? (plus any closing markup), then whitespace, then
# something that can open a sentence — lowercase included ("… store. macOS
# ships one"). Abbreviations ("e.g. on") are the exception, filtered below;
# decimals ("3.5") have no space after the stop.
SENTENCE_END = re.compile(r"[.!?](?:\*\*|\*|`|\)|»|\")*(?=\s+[A-Za-z0-9`*\[(¿¡«\"])")


def sentences(text):
    out, start = [], 0
    for m in SENTENCE_END.finditer(text):
        word = text[start : m.start()].split()[-1:] or [""]
        if word[0].lower().lstrip("(*`").endswith(ABBREVIATIONS):
            continue
        after = text[m.end():].lstrip()[:1]
        if after.islower() and text[m.start()] in "?!":
            continue  # a quoted question mid-sentence: ("what came in today?") still…
        out.append(text[start : m.end()].strip())
        start = m.end()
    tail = text[start:].strip()
    if tail:
        out.append(tail)
    return out


def fragments(block):
    """Split a block into fragments, in reading order."""
    lines = list(block.lines)
    if block.kind == "quote":
        lines = [re.sub(r"^\s*>\s?", "", l) for l in lines]
    prefix = ""
    if block.kind == "item":
        m = LIST_ITEM.match(lines[0])
        prefix = m.group(0)
        lines = [lines[0][m.end():]] + [l.strip() for l in lines[1:]]
    out, i, text = [], 0, []

    def flush():
        joined = "\n".join(l.rstrip() for l in text).strip()
        text.clear()
        for s in sentences(joined) if joined else []:
            out.append(Fragment("text", s))

    while i < len(lines):
        s = lines[i].strip()
        if s.startswith("```"):
            flush()
            j = i + 1
            while j < len(lines) and not lines[j].strip().startswith("```"):
                j += 1
            out.append(Fragment("code", "\n".join(l.strip() if block.kind == "item" else l
                                                  for l in lines[i : j + 1])))
            i = j + 1
        elif s.startswith("|"):
            flush()
            rows = []
            while i < len(lines) and lines[i].strip().startswith("|"):
                rows.append(lines[i].strip())
                i += 1
            for n, r in enumerate(rows):
                if re.fullmatch(r"\|[\s:|-]+\|", r):
                    continue  # the |---| separator is layout, not content
                out.append(Fragment("header" if n == 0 else "row", r))
        elif not s:
            flush()
            i += 1
        else:
            text.append(lines[i])
            i += 1
    flush()
    if out and prefix:
        out[0].prefix = prefix
    return out


def plain(md):
    """What a reader sees of a markdown span: no emphasis, code ticks or link
    targets, whitespace squashed. Checks quote the docs in this form."""
    s = re.sub(r"<!--.*?-->", "", md, flags=re.S)
    s = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", s)
    s = re.sub(r"\*\*|`|(?<![\w*])\*(?=\S)|(?<=\S)\*(?![\w*])|^>\s?", "", s, flags=re.M)
    return " ".join(s.split())


def locate(block, phrase):
    """Indices of the fragments a quoted phrase overlaps; [] if it is not in
    the block (the check is quoting text the docs no longer contain)."""
    frs = fragments(block)
    spans, text = [], ""
    for f in frs:
        t = plain(f.text)
        start = len(text) + (1 if text else 0)
        text = f"{text} {t}" if text else t
        spans.append((start, len(text)))
    needle = plain(phrase)
    at = text.find(needle) if needle else -1
    if at < 0:
        return []
    end = at + len(needle)
    return [n for n, (a, b) in enumerate(spans) if a < end and at < b]


def load_catalog():
    import tomllib

    with CATALOG.open("rb") as f:
        return tomllib.load(f)
