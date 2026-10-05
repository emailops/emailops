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
import sqlite3
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


class InvoiceReadStateDoesNotDependOnDrawOrder(unittest.TestCase):
    # Chat evals ask for the oldest unread email and for unread BorgBase mail.
    # Read state used to be a random draw, so three demo threads added elsewhere
    # in the generator shifted it and those evals failed on a correct answer.

    def test_the_unread_invoices_stay_unread_whatever_the_draw(self):
        for subject in gen.UNREAD_INVOICES_EN:
            self.assertFalse(gen.invoice_is_read(subject, "en", roll=0.0), subject)

    def test_every_other_english_invoice_is_read_whatever_the_draw(self):
        self.assertTrue(gen.invoice_is_read("Fly.io invoice — March 2026", "en", roll=0.99))

    def test_the_evals_unread_invoices_are_listed(self):
        self.assertEqual(
            gen.UNREAD_INVOICES_EN,
            {
                "BorgBase invoice for January 2026",
                "Your Hetzner Cloud invoice for February 2026",
                "BorgBase invoice for April 2026",
            },
        )


class VerificationFixturesAreSeeded(unittest.TestCase):
    """Rows the verification sweep and the chat evals drive: a trashed email and
    a junk-marked one that retrieval must leave out, an email with a remote
    image, a stored attachment of a type that can run code, and a draft with a
    table. They must survive a rebuild with the same ids and be addable to an
    existing demo DB without duplicating anything."""

    def _db(self):
        import sqlite3

        conn = sqlite3.connect(":memory:")
        conn.executescript(
            """
            CREATE TABLE emails (id TEXT PRIMARY KEY, account_id TEXT, thread_id TEXT, message_id TEXT,
                subject TEXT, sender TEXT, sender_email TEXT, sender_domain TEXT, recipients_json TEXT,
                cc_json TEXT, snippet TEXT, timestamp INTEGER, is_read INTEGER, is_deleted INTEGER,
                triage_status TEXT, category TEXT, mailbox TEXT, raw_json TEXT, created_at INTEGER,
                is_starred INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE email_bodies (email_id TEXT PRIMARY KEY, body TEXT);
            CREATE VIRTUAL TABLE emails_fts USING fts5(email_id UNINDEXED, subject, sender, body);
            CREATE TABLE email_tags (email_id TEXT, tag_type TEXT, tag_value TEXT, confidence REAL,
                created_at INTEGER, PRIMARY KEY (email_id, tag_type));
            CREATE TABLE email_headers (email_id TEXT PRIMARY KEY, account_id TEXT NOT NULL,
                from_raw TEXT, list_id TEXT, list_unsubscribe TEXT, list_unsubscribe_post TEXT,
                precedence TEXT, received_count INTEGER NOT NULL DEFAULT 0, captured_at INTEGER NOT NULL);
            CREATE TABLE email_junk (email_id TEXT PRIMARY KEY, account_id TEXT, spam_score REAL,
                phish_score REAL, gray_score REAL, band TEXT, primary_kind TEXT, reasons_json TEXT,
                method TEXT, model_version INTEGER, scored_at INTEGER, user_override TEXT, overridden_at INTEGER);
            CREATE TABLE email_attachment_meta (id TEXT PRIMARY KEY, email_id TEXT, account_id TEXT,
                provider_attachment_id TEXT, filename TEXT, mime_type TEXT, file_size INTEGER,
                file_path TEXT, inline_data TEXT);
            CREATE TABLE drafts (id TEXT PRIMARY KEY, email_id TEXT, account_id TEXT, to_addresses_json TEXT,
                subject TEXT, body TEXT, ai_generated INTEGER, status TEXT, created_at INTEGER,
                updated_at INTEGER, provider_draft_id TEXT, cc_addresses_json TEXT, body_html TEXT,
                provider_message_id TEXT, dirty INTEGER);
            """
        )
        return conn

    def _seeded(self):
        import tempfile

        conn = self._db()
        demo_dir = pathlib.Path(tempfile.mkdtemp())
        gen.insert_verification_fixtures(conn, gen.LOCALE_EN, demo_dir)
        return conn, demo_dir

    def _one(self, conn, sql, *params):
        return conn.execute(sql, params).fetchone()

    def test_the_trashed_quote_is_deleted_but_still_indexed(self):
        conn, _ = self._seeded()
        deleted = conn.execute("SELECT id, subject FROM emails WHERE is_deleted = 1").fetchall()
        self.assertEqual([subject for _, subject in deleted], ["Larkspur Freight renewal quote"])
        # Still in the keyword index, as after a delete in the app: retrieval has
        # to filter it out, the index does not do it for free.
        self.assertIsNotNone(self._one(conn, "SELECT 1 FROM emails_fts WHERE email_id = ?", deleted[0][0]))
        self.assertIn("3100 EUR", self._one(conn, "SELECT body FROM emails_fts WHERE email_id = ?", deleted[0][0])[0])
        live = self._one(conn, "SELECT b.body FROM emails e JOIN email_bodies b ON b.email_id = e.id "
                               "WHERE e.is_deleted = 0 AND e.subject = 'Corrected Larkspur Freight renewal quote'")
        self.assertIn("4200 EUR", live[0])

    def test_the_lookalike_billing_notice_is_marked_as_junk_by_the_user(self):
        conn, _ = self._seeded()
        row = self._one(conn, "SELECT j.band, j.primary_kind, j.user_override, e.mailbox, e.is_deleted "
                              "FROM email_junk j JOIN emails e ON e.id = j.email_id")
        # Scored below the junk band: only the user's own mark hides it, which is
        # the branch of the exclusion rule a scored-junk row would not exercise.
        self.assertEqual(row, ("uncertain", "phishing", "junk", "inbox", 0))
        chip = self._one(conn, "SELECT t.tag_value FROM email_tags t JOIN email_junk j ON j.email_id = t.email_id "
                               "WHERE t.tag_type = 'junk'")
        self.assertEqual(chip, ("phishing",))

    def test_one_email_carries_a_remote_image(self):
        conn, _ = self._seeded()
        bodies = [b for (b,) in conn.execute("SELECT body FROM email_bodies") if '<img src="https://' in b]
        self.assertEqual(len(bodies), 1)

    def test_the_attachments_that_can_run_code_are_stored_the_way_sync_stores_them(self):
        # A web page, kept inline as IMAP sync keeps small parts (the app previews
        # it itself, in a sandbox), and a shortcut stored on disk, which only the
        # OS can open: the one that asks for confirmation.
        import base64

        conn, demo_dir = self._seeded()
        rows = conn.execute(
            "SELECT filename, mime_type, provider_attachment_id, file_path, inline_data "
            "FROM email_attachment_meta WHERE mime_type NOT LIKE 'image/%' ORDER BY filename").fetchall()
        shortcut, page = rows
        self.assertEqual(shortcut[:3], ("larkspur-client-portal.webloc", "application/octet-stream", ""))
        self.assertFalse(pathlib.PurePosixPath(shortcut[3]).is_absolute(), "stored paths are relative to the data dir")
        self.assertIn("https://example.com/", (demo_dir / shortcut[3]).read_text(encoding="utf-8"))
        self.assertEqual(page[:4], ("larkspur-renewal-terms.html", "text/html", "INLINE::larkspur-renewal-terms.html", None))
        self.assertIn("renewal terms", base64.b64decode(page[4]).decode("utf-8"))

    def test_one_image_attachment_opens_in_the_lightbox(self):
        # The sweep opens it to prove no conversation shortcut acts behind the
        # image viewer. Kept inline, like a small IMAP part, and a real PNG.
        import base64

        conn, _ = self._seeded()
        rows = conn.execute(
            "SELECT filename, provider_attachment_id, inline_data FROM email_attachment_meta "
            "WHERE mime_type = 'image/png'").fetchall()
        self.assertEqual(len(rows), 1)
        filename, provider_id, data = rows[0]
        self.assertEqual(provider_id, f"INLINE::{filename}")
        self.assertTrue(base64.b64decode(data).startswith(b"\x89PNG\r\n\x1a\n"))

    def test_one_draft_holds_a_table(self):
        conn, _ = self._seeded()
        subject, html, status, dirty = self._one(conn, "SELECT subject, body_html, status, dirty FROM drafts")
        self.assertEqual((subject, status, dirty), ("Milestone dates (table)", "draft", 0))
        self.assertEqual(html.count("<tr>"), 3)

    def test_the_newsletter_offers_one_click_unsubscribe_to_a_reserved_host(self):
        # The Unsubscribe dialog and its oracle need RFC 8058 headers; the URL
        # is on `.example` (RFC 2606), so even a stray POST can reach no one.
        conn, _ = self._seeded()
        rows = conn.execute(
            "SELECT e.subject, h.list_unsubscribe, h.list_unsubscribe_post FROM email_headers h "
            "JOIN emails e ON e.id = h.email_id WHERE h.list_unsubscribe IS NOT NULL").fetchall()
        self.assertEqual(len(rows), 1)
        subject, header, post = rows[0]
        self.assertEqual(subject, gen.FIXTURE_REMOTE_IMAGE_SUBJECT)
        self.assertIn("<https://harborlight-weekly.example/", header)
        self.assertIn("<mailto:", header)
        self.assertEqual(post, "List-Unsubscribe=One-Click")

    def test_one_thread_is_starred(self):
        conn, _ = self._seeded()
        starred = [s for (s,) in conn.execute("SELECT subject FROM emails WHERE is_starred = 1")]
        self.assertEqual(starred, [gen.FIXTURE_STARRED_SUBJECT])

    def test_one_email_is_archived(self):
        conn, _ = self._seeded()
        archived = conn.execute("SELECT subject, is_read FROM emails WHERE mailbox = 'archive'").fetchall()
        self.assertEqual(archived, [(gen.FIXTURE_ARCHIVED_SUBJECT, 1)])

    def test_seeding_twice_changes_nothing(self):
        conn, demo_dir = self._seeded()
        counts = lambda: [conn.execute(f"SELECT COUNT(*) FROM {t}").fetchone()[0]
                          for t in ("emails", "emails_fts", "email_junk", "email_attachment_meta", "drafts",
                                    "email_headers")]
        ids = lambda: sorted(r[0] for r in conn.execute("SELECT id FROM emails"))
        before, before_ids = counts(), ids()
        gen.insert_verification_fixtures(conn, gen.LOCALE_EN, demo_dir)
        self.assertEqual((counts(), ids()), (before, before_ids))


class TheDemoChatShowsItsReasoning(unittest.TestCase):
    """The docs say every chat answer has a Show reasoning panel, and the docs
    check proves it by opening a saved conversation. The demo mailbox had none:
    the check only passed while a chat saved by hand survived in a local demo
    DB, and failed after the next `make demo-db`. One answered question is now
    seeded, with a real trace (`demo_fixtures/chat_reasoning_en.json`)."""

    MARISOL = "marisol@farologistics.com"

    def _db(self, with_thread=True):
        import sqlite3

        conn = sqlite3.connect(":memory:")
        conn.executescript(
            """
            CREATE TABLE emails (id TEXT PRIMARY KEY, account_id TEXT, thread_id TEXT, subject TEXT,
                sender TEXT, sender_email TEXT, timestamp INTEGER);
            CREATE TABLE chat_conversations (id TEXT PRIMARY KEY, account_id TEXT NOT NULL, title TEXT NOT NULL,
                created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
            CREATE TABLE chat_messages (id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL, role TEXT NOT NULL,
                content TEXT NOT NULL, model TEXT, token_count INTEGER, latency_ms INTEGER, trace TEXT,
                created_at INTEGER NOT NULL, referenced_email_ids TEXT, referenced_draft_ids TEXT,
                prompt_content TEXT);
            CREATE TABLE chat_message_sources (message_id TEXT NOT NULL, citation_number INTEGER NOT NULL,
                email_id TEXT NOT NULL, relevance_score REAL, subject TEXT NOT NULL DEFAULT '',
                sender TEXT NOT NULL DEFAULT '', sender_email TEXT NOT NULL DEFAULT '',
                email_timestamp INTEGER NOT NULL DEFAULT 0, body_excerpt TEXT,
                PRIMARY KEY (message_id, citation_number));
            """
        )
        if with_thread:
            work = gen.LOCALE_EN.work.id
            subject = "Re: Production bug: orders stuck in 'processing'"
            conn.executemany(
                "INSERT INTO emails VALUES (?, ?, 'thread_bug', ?, ?, ?, ?)",
                [("e_first", work, subject[4:], "Marisol Vega", self.MARISOL, 100),
                 ("e_reply", work, subject, "Ulises", gen.LOCALE_EN.work.email, 200),
                 ("e_last", work, subject, "Marisol Vega", self.MARISOL, 300)])
        return conn

    def _assistant(self, conn):
        return conn.execute("SELECT id, content, trace FROM chat_messages WHERE role = 'assistant'").fetchone()

    def test_one_conversation_on_the_work_account_asks_a_question(self):
        conn = self._db()
        self.assertEqual(gen.insert_demo_chat(conn, gen.LOCALE_EN), 1)
        rows = conn.execute("SELECT account_id, title FROM chat_conversations").fetchall()
        self.assertEqual(len(rows), 1)
        account, title = rows[0]
        self.assertEqual(account, gen.LOCALE_EN.work.id)
        # The docs case picks the saved conversation by its title ending in "?".
        self.assertTrue(title.endswith("?"), title)
        roles = [r for (r,) in conn.execute("SELECT role FROM chat_messages ORDER BY created_at")]
        self.assertEqual(roles, ["user", "assistant"])

    def test_the_answer_carries_the_trace_with_the_cited_email(self):
        import json

        conn = self._db()
        gen.insert_demo_chat(conn, gen.LOCALE_EN)
        _, content, trace = self._assistant(conn)
        self.assertNotIn("{EMAIL_ID}", trace)
        self.assertNotIn("{THREAD_ID}", trace)
        self.assertIn("id=e_last thread_id=thread_bug", trace)
        self.assertTrue(json.loads(trace)["steps"], "the reasoning panel walks the trace's steps")
        self.assertIn("(email://e_last)", content)

    def test_the_answer_cites_marisols_last_message(self):
        conn = self._db()
        gen.insert_demo_chat(conn, gen.LOCALE_EN)
        message_id, _, _ = self._assistant(conn)
        source = conn.execute(
            "SELECT message_id, citation_number, email_id, sender_email, email_timestamp "
            "FROM chat_message_sources").fetchall()
        self.assertEqual(source, [(message_id, 1, "e_last", self.MARISOL, 300)])

    def test_seeding_twice_changes_nothing(self):
        conn = self._db()
        gen.insert_demo_chat(conn, gen.LOCALE_EN)
        rows = lambda: [conn.execute(f"SELECT * FROM {t} ORDER BY 1").fetchall()
                        for t in ("chat_conversations", "chat_messages", "chat_message_sources")]
        before = rows()
        self.assertEqual(gen.insert_demo_chat(conn, gen.LOCALE_EN), 0)
        self.assertEqual(rows(), before)

    def test_a_mailbox_without_the_thread_gets_no_chat(self):
        # The Spanish demo has no such thread; a chat citing nothing would
        # render a broken source link.
        conn = self._db(with_thread=False)
        self.assertEqual(gen.insert_demo_chat(conn, gen.LOCALE_EN), 0)
        self.assertEqual(conn.execute("SELECT COUNT(*) FROM chat_messages").fetchone()[0], 0)


class TheSchemaComesFromThisCheckoutsMigrations(unittest.TestCase):
    """The demo DB used to copy its schema from the developer's production DB,
    which lags behind any branch that adds a migration: the app then failed to
    find the branch's new tables in the demo DB. The schema now comes from the
    app's own migrations (the `init_db` example), and the production DB is read
    only when `--prod-db` asks for it."""

    def _fake_init_db(self, version):
        calls = []

        def run(cmd, **kwargs):
            calls.append(cmd)
            data_dir = pathlib.Path(cmd[-1])
            data_dir.mkdir(parents=True, exist_ok=True)
            conn = sqlite3.connect(str(data_dir / "emailops.db"))
            conn.executescript(
                "CREATE TABLE refinery_schema_history (version INTEGER PRIMARY KEY, name TEXT,"
                " applied_on TEXT, checksum TEXT);"
                "CREATE TABLE blocked_senders (address TEXT PRIMARY KEY);"
            )
            conn.execute("INSERT INTO refinery_schema_history VALUES (?, 'x', '', '')", (version,))
            conn.commit()
            conn.close()

            class Done:
                returncode = 0
                stdout = f"{version}\n"

            return Done()

        return run, calls

    def _demo_db(self):
        import tempfile

        return pathlib.Path(tempfile.mkdtemp()) / "out" / "emailops.db"

    def test_the_production_db_is_not_read_unless_asked(self):
        self.assertIsNone(gen.build_parser().parse_args([]).prod_db)

    def test_the_latest_version_is_the_highest_migration_file(self):
        import tempfile

        d = pathlib.Path(tempfile.mkdtemp())
        for name in ("V001__init.sql", "V012__later.sql", "V009__mid.sql", "README.md"):
            (d / name).write_text("", encoding="utf-8")
        self.assertEqual(gen.latest_migration_version(d), 12)

    def test_the_repo_migrations_are_found(self):
        self.assertGreaterEqual(gen.latest_migration_version(), 34)

    def test_the_migrated_db_is_copied_into_place(self):
        latest = gen.latest_migration_version()
        run, calls = self._fake_init_db(latest)
        demo_db = self._demo_db()
        self.assertEqual(gen.migrate_schema(demo_db, run=run), latest)
        self.assertIn("init_db", calls[0])
        conn = sqlite3.connect(str(demo_db))
        tables = {r[0] for r in conn.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        self.assertIn("blocked_senders", tables)
        self.assertEqual(conn.execute("SELECT MAX(version) FROM refinery_schema_history").fetchone()[0], latest)

    def test_an_existing_demo_db_is_replaced(self):
        demo_db = self._demo_db()
        demo_db.parent.mkdir(parents=True)
        old = sqlite3.connect(str(demo_db))
        old.execute("CREATE TABLE stale (x)")
        old.commit()
        old.close()
        run, _ = self._fake_init_db(gen.latest_migration_version())
        gen.migrate_schema(demo_db, run=run)
        conn = sqlite3.connect(str(demo_db))
        tables = {r[0] for r in conn.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        self.assertNotIn("stale", tables)

    def test_a_db_behind_the_migrations_is_refused(self):
        run, _ = self._fake_init_db(gen.latest_migration_version() - 1)
        with self.assertRaises(SystemExit):
            gen.migrate_schema(self._demo_db(), run=run)
