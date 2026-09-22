"""Fragments are the unit the docs report colours: one sentence, one table
row or one code block. Run: uv run --no-project --with pytest pytest scripts/tests"""

import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
from docs_claims_lib import Block, blocks, fragments, locate, plain  # noqa: E402


def para(text, kind="paragraph"):
    return Block(kind, 1, text.splitlines(), "", "x")


def texts(block):
    return [f.text for f in fragments(block)]


def test_a_paragraph_splits_into_its_sentences():
    b = para("The wizard has four steps. It takes a couple of minutes.")
    assert texts(b) == ["The wizard has four steps.", "It takes a couple of minutes."]


def test_version_numbers_and_abbreviations_do_not_end_a_sentence():
    b = para("Qwen 3.5 4B is the default, e.g. on a 16 GB Mac. Nothing else.")
    assert texts(b) == ["Qwen 3.5 4B is the default, e.g. on a 16 GB Mac.", "Nothing else."]


def test_a_sentence_may_wrap_over_several_source_lines():
    b = para("The first time you open EmailOps a wizard\nruns. It is short.")
    assert texts(b) == ["The first time you open EmailOps a wizard\nruns.", "It is short."]


def test_bold_and_code_spans_do_not_hide_a_sentence_boundary():
    b = para("Open **Settings → AI**. Then pick `qwen`. Done.")
    assert texts(b) == ["Open **Settings → AI**.", "Then pick `qwen`.", "Done."]


def test_a_list_item_drops_its_bullet():
    b = para("- **Keep model loaded** — resident time. `0` evicts it.", kind="item")
    fr = fragments(b)
    assert [f.text for f in fr] == ["**Keep model loaded** — resident time.", "`0` evicts it."]
    assert fr[0].prefix == "- "


def test_each_table_row_is_one_fragment_and_the_header_is_structure():
    b = para("Models:\n\n| Model | Size |\n|---|---|\n| A | 1 GB |\n| B | 2 GB |")
    fr = fragments(b)
    assert [(f.kind, f.text) for f in fr] == [
        ("text", "Models:"), ("header", "| Model | Size |"), ("row", "| A | 1 GB |"), ("row", "| B | 2 GB |")]


def test_a_code_block_is_one_fragment():
    b = para("Run it:\n\n```bash\nemailops-cli doctor\nemailops-cli accounts\n```")
    fr = fragments(b)
    assert [f.kind for f in fr] == ["text", "code"]
    assert fr[1].text == "```bash\nemailops-cli doctor\nemailops-cli accounts\n```"


def test_plain_strips_markup_the_reader_does_not_see():
    assert plain("Open **Settings → AI**, pick `qwen` and see [the catalog](../x/#y).") == \
        "Open Settings → AI, pick qwen and see the catalog."


def test_locate_maps_a_quoted_phrase_to_the_fragments_it_overlaps():
    b = para("A wizard of up to four steps runs — three if you choose a plain client. It takes minutes.")
    assert locate(b, "three if you choose a plain client") == [0]
    assert locate(b, "plain client. It takes") == [0, 1]
    assert locate(b, "not in the text") == []


def test_locate_ignores_markup_and_line_wraps():
    b = para("Pick **Use\nAI** now. Then go.")
    assert locate(b, "Pick Use AI now") == [0]


def test_generated_region_markers_are_transparent(tmp_path):
    p = tmp_path / "page.md"
    p.write_text("<!-- claim:a -->\nModels:\n\n<!-- generated:model-catalog -->\n| M | S |\n|---|---|\n"
                 "| A | 1 |\n<!-- /generated:model-catalog -->\n\n<!-- claim:b -->\nAfter.\n")
    got = blocks(p)
    assert [(b.claim, b.kind) for b in got] == [("a", "paragraph"), ("b", "paragraph")]
    assert [f.kind for f in fragments(got[0])] == ["text", "header", "row"]


def test_headings_are_emitted_only_when_asked(tmp_path):
    p = tmp_path / "page.md"
    p.write_text("---\ntitle: T\n---\n\n## Intro {#intro}\n\n<!-- claim:a -->\nHello.\n")
    assert [b.kind for b in blocks(p)] == ["paragraph"]
    got = blocks(p, with_headings=True)
    assert [(b.kind, b.text) for b in got] == [("heading", "Intro"), ("paragraph", "Hello.")]
    assert got[0].level == 2
