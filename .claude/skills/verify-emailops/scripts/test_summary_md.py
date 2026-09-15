"""Tests for summary_md.py, the Markdown summary committed after each verification run.

    cd .claude/skills/verify-emailops/scripts && python3 -m unittest test_summary_md
"""
import pathlib, tempfile, unittest

import summary_md


def rec(name, status, feature="Chat", typ="eval", detail="", category=None):
    evidence = {"category": category} if category else {}
    return {"feature": feature, "type": typ, "name": name, "status": status, "detail": detail,
            "duration_ms": 1, "desc": "", "evidence": evidence}


def run(records, run_dir="/repo/src-tauri/reports/verify/20260915-110600-full"):
    meta = {"started": "2026-09-15T11:06:00", "finished": "2026-09-15T11:20:30", "tier": "full",
            "commit": "f911f3d docs: record a decision (2026-09-15 10:57:31 +0200)",
            "branch": "feature/ui-verification", "dirty": [], "run_dir": run_dir,
            "evals": {"model": "m1", "judge_model": "j1"}}
    return {"meta": meta, "records": records}


class SummaryTest(unittest.TestCase):
    def test_header_names_commit_start_and_duration(self):
        md = summary_md.summarize(run([]), None)
        self.assertIn("`f911f3d`", md)
        self.assertIn("15/09/2026 11:06", md)
        self.assertIn("14.5 min", md)

    def test_totals_count_each_status(self):
        md = summary_md.summarize(run([rec("a", "ok"), rec("b", "ok"), rec("c", "fail"), rec("d", "skip")]), None)
        self.assertIn("| 2 | 1 | 1 | 0 |", md)

    def test_failure_line_has_category_and_only_the_clipped_first_detail_line(self):
        detail = "x" * 300 + "\nsecond line"
        md = summary_md.summarize(run([rec("ts_case (smoke)", "fail", detail=detail, category="thread_summary")]), None)
        line = next(l for l in md.splitlines() if "ts_case (smoke)" in l)
        self.assertIn("thread_summary", line)
        self.assertNotIn("second line", md)
        self.assertLess(len(line), 300)

    def test_delta_lists_now_passing_and_now_failing_against_the_previous_run(self):
        prev = run([rec("fixed", "fail"), rec("broken", "ok")], run_dir="/repo/src-tauri/reports/verify/20260914-232040-full")
        md = summary_md.summarize(run([rec("fixed", "ok"), rec("broken", "fail")]), prev)
        self.assertIn("20260914-232040-full", md)
        now_passing, rest = md.split("### Now passing")[1].split("### Now failing")
        self.assertIn("fixed", now_passing)
        self.assertNotIn("broken", now_passing)
        self.assertIn("broken", rest)

    def test_without_a_previous_run_the_delta_says_so(self):
        md = summary_md.summarize(run([rec("a", "ok")]), None)
        self.assertIn("No previous full run", md)

    def test_refuses_a_private_run(self):
        # Private runs carry real mailbox content; their summary must never reach the repo.
        data = run([], run_dir="/repo/src-tauri/reports/verify-private/20260915-110600-private")
        with self.assertRaises(ValueError):
            summary_md.summarize(data, None)

    def test_file_name_is_run_stamp_and_short_commit(self):
        self.assertEqual(summary_md.file_name(run([])), "20260915-110600-f911f3d.md")

    def test_previous_run_is_the_latest_older_full_run(self):
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            for name in ("20260913-100000-full", "20260914-232040-full", "20260915-105800-gpu-oom", "20260915-110600-full"):
                (root / name).mkdir()
                (root / name / "results.json").write_text("{}")
            self.assertEqual(summary_md.previous_run(root / "20260915-110600-full"), root / "20260914-232040-full")


if __name__ == "__main__":
    unittest.main()
