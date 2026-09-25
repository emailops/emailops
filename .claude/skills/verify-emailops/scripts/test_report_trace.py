"""Tests for report_trace.py, the eval-case rendering used by report_all.py.

    cd .claude/skills/verify-emailops/scripts && python3 -m unittest test_report_trace
"""
import unittest

import report_trace


def trace(**over):
    t = {
        "route": {"classifier": "heuristic", "matchedKeywords": ["latest"], "mode": "tools_first", "reason": "heuristic matched: latest"},
        "llmCalls": [
            {"kind": "planner", "round": -2, "latencyMs": 5470, "prefillMs": 1715, "toolCallsRequested": 0, "failed": False, "output": "planner: search"},
            {"kind": "tool_round", "round": 0, "latencyMs": 900, "toolCallsRequested": 1, "failed": False, "input": "[system] line one\nline two", "output": "tool_call: search_emails({})"},
            {"kind": "final_stream", "round": 1, "latencyMs": 300, "toolCallsRequested": 0, "failed": True},
        ],
        "toolCalls": [{"name": "search_emails", "round": 0, "arguments": {"from": "Sara", "limit": 5}, "elapsedMs": 12, "resultChars": 52, "resultPreview": "No matching emails."}],
        "steps": [
            {"type": "route"},
            {"type": "llm", "index": 0, "kvCache": {"cached": 2267, "total": 2286, "pct": 99}, "cacheAction": {"kind": "cold-fresh", "detail": "cold prefill · no anchor seeded"}},
            {"type": "llm", "index": 1},
            {"type": "tool", "index": 0},
            {"type": "llm", "index": 2},
        ],
    }
    t.update(over)
    return t


class FlowTest(unittest.TestCase):
    def test_flow_names_every_step_in_order(self):
        self.assertEqual(
            report_trace.flow_summary(trace()),
            "route: heuristic (matched: latest) → planner → llm round 0 → search_emails → answer",
        )

    def test_flow_includes_retrieval_and_guides(self):
        t = trace(steps=[{"type": "route"}, {"type": "retrieval"}, {"type": "help"}],
                  route={"classifier": "planner", "matchedKeywords": [], "mode": "rag_first", "reason": ""},
                  help={"candidates": 24, "included": 2, "elapsedMs": 7})
        self.assertEqual(report_trace.flow_summary(t), "route: planner → RAG retrieval → guides (2 of 24 sections)")

    def test_a_trace_without_steps_has_no_flow(self):
        self.assertIsNone(report_trace.flow_summary(trace(steps=[])))
        self.assertIsNone(report_trace.flow_summary(None))


class StepDetailTest(unittest.TestCase):
    def test_llm_detail_carries_timings_cache_and_tool_calls(self):
        t = trace()
        self.assertEqual(
            report_trace.step_detail(t, t["steps"][1]),
            "5470 ms · prefill 1715 ms · KV cache 2267/2286 tok (99%) · cold prefill · no anchor seeded",
        )
        self.assertEqual(report_trace.step_detail(t, t["steps"][2]), "900 ms · 1 tool call")
        self.assertEqual(report_trace.step_detail(t, t["steps"][4]), "300 ms · FAILED")

    def test_route_and_tool_detail(self):
        t = trace()
        self.assertEqual(report_trace.step_detail(t, t["steps"][0]), "tools_first · heuristic matched: latest")
        self.assertEqual(report_trace.step_detail(t, t["steps"][3]), "12 ms · 52 chars")


class StepsHtmlTest(unittest.TestCase):
    def test_prompt_text_keeps_real_line_breaks_not_json_escapes(self):
        out = report_trace.steps_html(trace())
        self.assertIn("[system] line one\nline two", out)
        self.assertNotIn("\\n", out)

    def test_tool_arguments_are_pretty_printed_and_escaped(self):
        out = report_trace.steps_html(trace(toolCalls=[{"name": "search_emails", "round": 0, "arguments": {"q": "<b>"}, "elapsedMs": 1, "resultChars": 3, "resultPreview": "a&b"}]))
        self.assertIn('{\n  &quot;q&quot;: &quot;&lt;b&gt;&quot;\n}', out)
        self.assertIn("a&amp;b", out)


class JudgeChecksTest(unittest.TestCase):
    def test_each_scored_metric_becomes_a_check_against_the_threshold(self):
        jr = {"model": "judge-m", "threshold": 0.7, "scores": {"answerRelevancy": 1.0, "faithfulness": 0.4, "contextualRecall": None, "error": None, "rationale": "Cites the wrong email."}}
        rows = report_trace.judge_checks(jr)
        self.assertEqual([(r["name"], r["expected"], r["actual"], r["passed"]) for r in rows], [
            ("juez · answerRelevancy", "≥ 0.70", "1.00", True),
            ("juez · faithfulness", "≥ 0.70", "0.40", False),
        ])
        self.assertTrue(all(r["detail"] == "Cites the wrong email." for r in rows))

    def test_a_judge_error_is_a_failed_check(self):
        rows = report_trace.judge_checks({"model": "j", "threshold": 0.7, "scores": {"error": "timeout"}})
        self.assertEqual([(r["name"], r["passed"], r["detail"]) for r in rows], [("juez", False, "timeout")])

    def test_no_judge_report_means_no_rows(self):
        self.assertEqual(report_trace.judge_checks(None), [])


if __name__ == "__main__":
    unittest.main()
