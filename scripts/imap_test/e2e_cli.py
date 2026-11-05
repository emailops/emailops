"""End-to-end check of an IMAP account through emailops-cli.

Drives the real CLI binary against the local Dovecot + Mailpit servers of
scripts/imap_test.sh, as a fresh user in a throwaway data dir:

  seed 3 messages over IMAP -> accounts add imap -> sync -> emails / search /
  show -> compose --send (Mailpit receives it, Sent gets a copy) -> sync again
  (nothing duplicated).

Usage (via `make imap-e2e`, which starts the servers and builds the CLI):
  python3 -I scripts/imap_test/e2e_cli.py <emailops-cli binary> <state dir>

Reads the EMAILOPS_IMAP_LIVE_* / EMAILOPS_IMAP_TEST_CA env vars that
`scripts/imap_test.sh env` prints. Synthetic data only. Stdlib only.
"""

import email.utils
import imaplib
import json
import os
import ssl
import subprocess
import sys
import time
import urllib.parse
import urllib.request
import uuid

CHECKS = []


def check(name, ok, detail=""):
    CHECKS.append((name, bool(ok), detail))
    mark = "ok  " if ok else "FAIL"
    print(f"  {mark} {name}" + (f" — {detail}" if detail and not ok else ""))


def imap_session(host, port, user, password, cafile):
    context = ssl.create_default_context(cafile=cafile)
    session = imaplib.IMAP4(host, port)
    session.starttls(ssl_context=context)
    session.login(user, password)
    return session


def seed(session, user, n, subject):
    message_id = f"<e2e-{n}-{uuid.uuid4().hex}@example.test>"
    raw = (
        f"Message-ID: {message_id}\r\n"
        f"Date: {email.utils.formatdate(localtime=False)}\r\n"
        "From: Sender Example <sender@example.test>\r\n"
        f"To: {user}\r\n"
        f"Subject: {subject}\r\n"
        "MIME-Version: 1.0\r\n"
        "Content-Type: text/plain; charset=utf-8\r\n"
        "\r\n"
        f"Body of {subject}. Synthetic test message.\r\n"
    ).encode()
    status, _ = session.append("INBOX", None, None, raw)
    if status != "OK":
        raise RuntimeError(f"APPEND failed: {status}")
    return message_id


class Cli:
    def __init__(self, binary, data_dir, account=None):
        self.binary = binary
        self.data_dir = data_dir
        self.account = account

    def run(self, *args):
        cmd = [self.binary, "--json", "--quiet", "--data-dir", self.data_dir]
        if self.account and args[0] not in ("accounts",):
            cmd += ["--account", self.account]
        cmd += list(args)
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=300)
        try:
            envelope = json.loads(proc.stdout)
        except json.JSONDecodeError:
            envelope = {"ok": False, "error": {"message": proc.stdout[-500:] + proc.stderr[-1500:]}}
        envelope["_exit"] = proc.returncode
        return envelope


def items(data, *keys):
    """The list inside an envelope's data, whatever key it sits under."""
    if isinstance(data, list):
        return data
    for key in keys:
        if isinstance(data, dict) and isinstance(data.get(key), list):
            return data[key]
    return []


def mailpit_has(base, subject, within=15):
    query = urllib.parse.quote(f'subject:"{subject}"')
    deadline = time.time() + within
    while time.time() < deadline:
        with urllib.request.urlopen(f"{base}/api/v1/search?query={query}") as resp:
            if json.load(resp).get("messages_count", 0) > 0:
                return True
        time.sleep(0.5)
    return False


def main():
    binary, state = sys.argv[1], sys.argv[2]
    host = os.environ["EMAILOPS_IMAP_LIVE_HOST"]
    port = int(os.environ["EMAILOPS_IMAP_LIVE_PORT"])
    smtp_port = os.environ["EMAILOPS_IMAP_LIVE_SMTP_PORT"]
    password = os.environ["EMAILOPS_IMAP_LIVE_PASSWORD"]
    cafile = os.environ["EMAILOPS_IMAP_TEST_CA"]
    mailpit = os.environ["EMAILOPS_IMAP_LIVE_MAILPIT"]

    run_id = uuid.uuid4().hex[:10]
    user = f"e2e-{run_id}@example.test"
    data_dir = os.path.join(state, f"e2e-{run_id}")
    os.makedirs(data_dir)
    print(f"user {user}, data dir {data_dir}")

    session = imap_session(host, port, user, password, cafile)
    subjects = [f"Invoice {run_id}", f"Meeting notes {run_id}", f"Travel plan {run_id}"]
    for n, subject in enumerate(subjects):
        seed(session, user, n, subject)
    session.logout()

    cli = Cli(binary, data_dir)
    added = cli.run(
        "accounts", "add", "imap",
        "--host", host, "--port", str(port), "--email", user, "--password", password,
        "--smtp-host", host, "--smtp-port", smtp_port, "--name", "E2E Owner",
    )
    check("accounts add imap", added.get("ok"), json.dumps(added.get("error")))
    if not added.get("ok"):
        return finish()
    cli.account = user

    synced = cli.run("sync")
    check("sync", synced.get("ok"), json.dumps(synced.get("error")))

    listed = cli.run("emails", "--limit", "50")
    emails = items(listed.get("data"), "emails", "items")
    got = sorted(e.get("subject", "") for e in emails)
    check("emails lists the 3 seeded messages", got == sorted(subjects), f"got {got}")

    hits = cli.run("search", f"Invoice {run_id}")
    found = items(hits.get("data"), "hits", "results", "emails", "items")
    check("search finds the invoice", hits.get("ok") and len(found) >= 1, json.dumps(hits)[:400])

    invoice = next((e for e in emails if e.get("subject") == subjects[0]), None)
    if invoice:
        shown = cli.run("show", invoice["id"])
        text = json.dumps(shown.get("data"))
        check("show returns the body", shown.get("ok") and "Synthetic test message" in text, text[:300])
        check("show keeps the sender", "sender@example.test" in text, text[:300])

    sent_subject = f"E2E reply {run_id}"
    composed = cli.run(
        "compose", "--to", "friend@example.test", "--subject", sent_subject,
        "--body", "Synthetic reply from the e2e run.", "--send",
    )
    check("compose --send", composed.get("ok"), json.dumps(composed.get("error")))
    check("Mailpit received the message", mailpit_has(mailpit, sent_subject))

    session = imap_session(host, port, user, password, cafile)
    session.select("Sent", readonly=True)
    _, data = session.search(None, "HEADER", "Subject", f'"{sent_subject}"')
    check("a copy was filed in Sent on the server", bool(data and data[0].split()))
    session.logout()

    again = cli.run("sync")
    check("second sync", again.get("ok"), json.dumps(again.get("error")))
    relisted = items(cli.run("emails", "--limit", "50").get("data"), "emails", "items")
    check("second sync duplicates nothing", len(relisted) == len(emails), f"{len(relisted)} vs {len(emails)}")

    return finish()


def finish():
    failed = [name for name, ok, _ in CHECKS if not ok]
    print(f"\n{len(CHECKS) - len(failed)}/{len(CHECKS)} checks passed")
    return 1 if failed or not CHECKS else 0


if __name__ == "__main__":
    sys.exit(main())
