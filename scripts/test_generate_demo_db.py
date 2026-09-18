"""Tests for the demo-data generator's id scheme.

    cd scripts && python3 -m unittest test_generate_demo_db

Run automatically by `make verify` (static layer). The point of these is one
regression: every id used to come from `uuid.uuid4()`, which reads `os.urandom`
and ignores the module's seed, so each `make demo-db` re-keyed the whole
mailbox. Six chat eval cases pinned to thread ids and four to email ids all
stopped matching, and they did it quietly — the cases still "ran" and reported
a generic answer failure.
"""
import ast
import pathlib
import unittest

import generate_demo_db as gen


class DemoIdIsAFunctionOfItsKey(unittest.TestCase):
    def test_same_key_always_gives_the_same_id(self):
        first = gen.demo_id("demo_", "acct", "a@b.com", "Subject", length=16)
        second = gen.demo_id("demo_", "acct", "a@b.com", "Subject", length=16)
        self.assertEqual(first, second)

    def test_a_different_key_gives_a_different_id(self):
        a = gen.demo_id("demo_", "acct", "a@b.com", "Subject")
        b = gen.demo_id("demo_", "acct", "a@b.com", "Other subject")
        self.assertNotEqual(a, b)

    def test_key_parts_cannot_run_together(self):
        # Without a separator ("ab", "c") and ("a", "bc") would hash the same,
        # so two different rows could collide into one id.
        self.assertNotEqual(gen.demo_id("x_", "ab", "c"), gen.demo_id("x_", "a", "bc"))

    def test_the_prefix_is_kept_and_the_length_is_exact(self):
        got = gen.demo_id("thread_", "k", length=12)
        self.assertTrue(got.startswith("thread_"))
        self.assertEqual(len(got), len("thread_") + 12)

    def test_an_id_does_not_depend_on_draw_order(self):
        # A seeded counter would fix regeneration but not this: inserting a row
        # would shift every id after it. Content addressing means an id depends
        # on nothing but its own key.
        gen.RNG.random()
        after_unrelated_draws = gen.demo_id("demo_", "acct", "a@b.com", "Subject", length=16)
        self.assertEqual(
            after_unrelated_draws,
            gen.demo_id("demo_", "acct", "a@b.com", "Subject", length=16),
        )


class NoUnseededRandomnessLeaksBackIn(unittest.TestCase):
    def test_the_generator_never_calls_uuid4(self):
        # Parsed, not grepped: the prose in `demo_id`'s own docstring names
        # uuid.uuid4() to explain why it is gone, and a regex would trip on it.
        tree = ast.parse(pathlib.Path(gen.__file__).read_text(encoding="utf-8"))
        calls = [
            node
            for node in ast.walk(tree)
            if isinstance(node, ast.Call)
            and isinstance(node.func, ast.Attribute)
            and node.func.attr == "uuid4"
        ]
        self.assertEqual(
            [f"line {c.lineno}" for c in calls],
            [],
            "uuid.uuid4() ignores the module seed — use demo_id() so eval cases "
            "can pin an id and have it survive the next `make demo-db`",
        )


if __name__ == "__main__":
    unittest.main()
