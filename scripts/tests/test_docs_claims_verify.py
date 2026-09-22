"""How validations land on fragments and what colour a fragment gets."""

import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
from docs_claims_lib import Block  # noqa: E402
from docs_claims_verify import app_validations, catalog_validations, color, fragment_model  # noqa: E402

BLOCK = Block("paragraph", 1, ["A wizard of up to four steps runs — three if you choose a plain client.",
                               "It takes a couple of minutes."], "", "start-intro-1")


def part(**kw):
    base = {"claim": "start-intro-1", "name": "pasos", "status": "ok", "detail": "4 y 3", "tag": "APP",
            "how": "cuenta los pasos", "where": "doc_claims.mjs:10", "read_doc": True, "proof": "behaviour",
            "covers": ["up to four steps", "three if you choose a plain client"]}
    return {**base, **kw}


def states(model):
    return [[v["state"] for v in f["validations"]] for f in model]


def test_colour_rules():
    assert color([{"state": "ok"}, {"state": "manual"}]) == "green"
    assert color([{"state": "ok"}, {"state": "fail"}]) == "red"
    assert color([{"state": "supported"}]) == "green"
    assert color([{"state": "contradicted"}]) == "red"
    for s in ("label", "fixed", "undeclared", "manual", "none", "pending", "skip", "insufficient"):
        assert color([{"state": s}]) == "yellow", s
    assert color([]) == "yellow"


def test_a_case_colours_only_the_sentences_it_quotes():
    model = fragment_model(BLOCK, app_validations(BLOCK, [part()], ran=True))
    assert states(model) == [["ok"], []]
    assert [f["color"] for f in model] == ["green", "yellow"]
    assert model[1]["tags"] == ["SIN"]


def test_a_case_quoting_text_the_docs_dropped_is_a_failure():
    v = app_validations(BLOCK, [part(covers=["up to five steps"])], ran=True)
    model = fragment_model(BLOCK, v)
    assert all(f["color"] == "red" for f in model)
    assert "up to five steps" in model[0]["validations"][0]["detail"]


def test_a_case_that_declares_nothing_is_undeclared_on_the_whole_block():
    model = fragment_model(BLOCK, app_validations(BLOCK, [part(covers=None)], ran=True))
    assert states(model) == [["undeclared"], ["undeclared"]]


def test_a_failing_case_that_declares_nothing_is_red_on_the_whole_block():
    model = fragment_model(BLOCK, app_validations(BLOCK, [part(covers=None, status="fail")], ran=True))
    assert [f["color"] for f in model] == ["red", "red"]


def test_a_case_that_never_read_the_docs_has_a_fixed_expectation():
    model = fragment_model(BLOCK, app_validations(BLOCK, [part(read_doc=False)], ran=True))
    assert states(model)[0] == ["fixed"]


def test_a_label_only_proof_is_yellow():
    model = fragment_model(BLOCK, app_validations(BLOCK, [part(proof="label")], ran=True))
    assert states(model)[0] == ["label"]


def test_app_checks_that_did_not_run_are_pending_over_the_block():
    model = fragment_model(BLOCK, app_validations(BLOCK, [], ran=False))
    assert states(model) == [["pending"], ["pending"]]


def test_an_app_check_no_phase_ran_is_a_failure():
    model = fragment_model(BLOCK, app_validations(BLOCK, [], ran=True))
    assert [f["color"] for f in model] == ["red", "red"]


def test_a_file_check_covers_the_sentence_it_quotes():
    entry = {"checks": [{"file": "README.md", "quoted": "couple of minutes"}]}
    v = catalog_validations(BLOCK, entry, file_check=lambda ch, text: (True, "ok"), test_status={})
    assert states(fragment_model(BLOCK, v)) == [[], ["ok"]]


def test_a_manual_entry_without_covers_spans_the_block_and_one_with_covers_does_not():
    whole = catalog_validations(BLOCK, {"checks": [{"manual": "why"}]}, file_check=None, test_status={})
    assert states(fragment_model(BLOCK, whole)) == [["manual"], ["manual"]]
    part_ = catalog_validations(BLOCK, {"checks": [{"manual": "why", "covers": ["couple of minutes"]}]},
                                file_check=None, test_status={})
    assert states(fragment_model(BLOCK, part_)) == [[], ["manual"]]


def test_a_passing_catalog_check_that_declares_nothing_is_undeclared():
    v = catalog_validations(BLOCK, {"checks": [{"tests": ["a::b"]}]}, file_check=None, test_status={"a::b": "ok"})
    assert states(fragment_model(BLOCK, v)) == [["undeclared"], ["undeclared"]]
    v = catalog_validations(BLOCK, {"checks": [{"tests": ["a::b"], "covers": ["couple of minutes"]}]},
                            file_check=None, test_status={"a::b": "ok"})
    assert states(fragment_model(BLOCK, v)) == [[], ["ok"]]


def test_tests_are_tagged_and_fail_when_missing():
    v = catalog_validations(BLOCK, {"checks": [{"tests": ["a::b"]}]}, file_check=None, test_status={"a::b": "missing"})
    model = fragment_model(BLOCK, v)
    assert model[0]["tags"] == ["TST"] and model[0]["color"] == "red"
