"""The docs report: the page itself, each fragment coloured and tagged."""

import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
from docs_report import actions, fragment_html, inline, page_html, summary  # noqa: E402


def v(state, tag="APP", **kw):
    base = {"tag": tag, "state": state, "how": "cuenta los pasos del asistente", "detail": "4 pasos",
            "where": "doc_claims.mjs:10", "fix": "", "evidence": {}}
    return {**base, **kw}


def frag(color, vs, text="A wizard of four steps runs.", kind="text"):
    tags = list(dict.fromkeys(x["tag"] for x in vs)) or ["SIN"]
    return {"kind": kind, "text": text, "prefix": "", "validations": vs, "color": color, "tags": tags}


PAGE = {"page": "getting-started.md", "title": "Primeros pasos", "items": [
    {"kind": "heading", "level": 2, "text": "The wizard", "line": 3},
    {"kind": "paragraph", "claim": "start-intro-1", "line": 5, "fragments": [
        frag("green", [v("ok")]),
        frag("red", [v("fail", detail="la app muestra 5", fix="Decir cinco pasos")], "It has five steps."),
        frag("yellow", [], "It takes minutes."),
    ]},
    {"kind": "paragraph", "claim": "start-2", "line": 9, "fragments": [
        frag("yellow", [v("undeclared", where="doc_claims.mjs:40", fix="Declarar covers")], "One."),
        frag("yellow", [v("undeclared", where="doc_claims.mjs:40", fix="Declarar covers")], "Two."),
    ]},
    {"kind": "table", "claim": "t", "line": 12, "fragments": [
        frag("none", [], "| Model | Size |", kind="header"),
        frag("green", [v("ok", tag="GEN")], "| A | 1 GB |", kind="row"),
    ]},
]}


def test_inline_markdown_is_rendered_and_html_escaped():
    assert inline("Open **Settings** and run `a<b` via [docs](../x/)") == \
        'Open <strong>Settings</strong> and run <code>a&lt;b</code> via <a href="#p-x">docs</a>'


def test_a_fragment_carries_its_colour_and_ends_in_its_tag():
    html = fragment_html(frag("red", [v("fail", fix="Decir cinco pasos")]), "f1")
    assert 'class="frag c-red"' in html
    assert html.index("A wizard of four steps runs.") < html.index("[APP]")


def test_the_tag_popover_explains_how_where_evidence_and_fix():
    html = fragment_html(frag("red", [v("fail", fix="Decir cinco pasos",
                                        evidence={"observed": "5 pasos", "shots": ["app/x.png"]})]), "f1")
    for s in ("cuenta los pasos del asistente", "doc_claims.mjs:10", "5 pasos", "app/x.png", "Decir cinco pasos"):
        assert s in html, s


def test_green_fragments_do_not_suggest_fixes():
    html = fragment_html(frag("green", [v("ok", fix="should not show")]), "f1")
    assert "Pasos sugeridos" not in html


def test_an_uncovered_fragment_is_tagged_and_says_what_to_do():
    html = fragment_html(frag("yellow", [], "It takes minutes."), "f1")
    assert "[SIN]" in html and "Ninguna comprobación" in html and "Pasos sugeridos" in html


def test_summary_counts_fragments_per_tag_and_colour():
    s = summary(PAGE)
    assert s["APP"] == {"green": 1, "yellow": 2, "red": 1}
    assert s["GEN"] == {"green": 1, "yellow": 0, "red": 0}
    assert s["SIN"] == {"green": 0, "yellow": 1, "red": 0}
    assert s["Total"] == {"green": 2, "yellow": 3, "red": 1}


def test_actions_put_errors_first_and_group_one_case_into_one_action():
    acts = actions(PAGE)
    assert acts[0]["severity"] == "red" and "Decir cinco pasos" in acts[0]["text"]
    undeclared = [a for a in acts if "doc_claims.mjs:40" in a["text"]]
    assert len(undeclared) == 1 and len(undeclared[0]["targets"]) == 2


def test_the_page_renders_as_the_docs_with_no_case_tables():
    html = page_html(PAGE, 0)
    assert "<h3" in html and "The wizard" in html
    assert "Acciones recomendadas" in html and "<table class=\"summary\"" in html
    assert html.count("<tr class=\"frag") == 1  # the data row; the header row is structure
    assert "class=\"cases\"" not in html
