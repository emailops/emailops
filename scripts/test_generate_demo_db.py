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


class MemoryFactsAreSearchable(unittest.TestCase):
    """The app keeps `memory_facts_fts` in step from Rust (the migration has a
    delete trigger only), so rows written straight into `memory_facts` are
    invisible to the chat's `<memory>` header. The demo facts sat there
    unindexed and `mem_borgbase_customer_number` failed on every run."""

    def _db(self):
        import sqlite3

        conn = sqlite3.connect(":memory:")
        conn.execute(
            """CREATE TABLE memory_facts (
                 id TEXT PRIMARY KEY, account_id TEXT, subject_kind TEXT, subject_key TEXT,
                 fact TEXT, source TEXT, source_email_id TEXT, confidence REAL, score REAL,
                 status TEXT, last_used_at INTEGER, created_at INTEGER, updated_at INTEGER,
                 domain TEXT, vigency TEXT, company TEXT)"""
        )
        conn.execute(
            "CREATE VIRTUAL TABLE memory_facts_fts USING fts5("
            "fact_id UNINDEXED, fact, subject_key, tokenize='porter unicode61')"
        )
        return conn

    def _indexed(self, conn, term):
        return [
            row[0]
            for row in conn.execute(
                "SELECT f.fact FROM memory_facts f JOIN memory_facts_fts fts ON fts.fact_id = f.id "
                "WHERE memory_facts_fts MATCH ?",
                (f'"{term}"',),
            )
        ]

    def test_every_seeded_fact_is_indexed(self):
        conn = self._db()
        gen.insert_memory_facts(conn, gen.LOCALE_EN)
        facts = conn.execute("SELECT COUNT(*) FROM memory_facts").fetchone()[0]
        indexed = conn.execute("SELECT COUNT(*) FROM memory_facts_fts").fetchone()[0]
        self.assertGreater(facts, 0)
        self.assertEqual(indexed, facts)
        self.assertTrue(any("BB-48213" in f for f in self._indexed(conn, "BorgBase")))

    def test_facts_appended_to_an_existing_db_are_indexed(self):
        conn = self._db()
        gen.append_memory_facts(conn, gen.LOCALE_EN)
        facts = conn.execute("SELECT COUNT(*) FROM memory_facts").fetchone()[0]
        indexed = conn.execute("SELECT COUNT(*) FROM memory_facts_fts").fetchone()[0]
        self.assertGreater(facts, 0)
        self.assertEqual(indexed, facts)


class ThreadMessagesAreAddressedToTheOtherSide(unittest.TestCase):
    """A message the owner sends goes to the counterparty. It used to list the
    owner as its own recipient, so research read "From: YOU, To: YOU" and
    could not tell a quote the user sent from one the user received."""

    def test_the_owners_messages_go_to_the_counterparty(self):
        captured = []
        original_insert, original_tags = gen.insert_email, gen.insert_tags
        gen.insert_email = lambda conn, **kw: captured.append(kw) or f"id{len(captured)}"
        gen.insert_tags = lambda *a, **kw: None
        try:
            thread = gen.Thread("Ana", "ana@client.example", "Quote", "primary",
                                [("me", "Here is my quote."), ("them", "Thanks!")])
            gen._insert_thread(None, gen.LOCALE_EN.work, thread)
        finally:
            gen.insert_email, gen.insert_tags = original_insert, original_tags
        self.assertEqual(captured[0]["recipient_email"], "ana@client.example")
        self.assertIsNone(captured[1].get("recipient_email"), "their message goes to the owner")


if __name__ == "__main__":
    unittest.main()
