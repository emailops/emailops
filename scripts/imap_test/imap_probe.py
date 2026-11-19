"""Seed and inspect a mailbox on the local test Dovecot (scripts/imap_test.sh).

  python3 -I imap_probe.py seed <user> <subject>...   deliver one message per subject
  python3 -I imap_probe.py where <user> <subject>     print {"folder": …, "flags": […]} as JSON

Reads EMAILOPS_IMAP_LIVE_HOST/PORT/PASSWORD and EMAILOPS_IMAP_TEST_CA.
Synthetic data only. Stdlib only.
"""

import email.utils
import imaplib
import json
import os
import ssl
import sys
import uuid

FOLDERS = ["INBOX", "Archive", "Junk", "Trash", "Sent"]


def session(user):
    context = ssl.create_default_context(cafile=os.environ["EMAILOPS_IMAP_TEST_CA"])
    imap = imaplib.IMAP4(os.environ["EMAILOPS_IMAP_LIVE_HOST"], int(os.environ["EMAILOPS_IMAP_LIVE_PORT"]))
    imap.starttls(ssl_context=context)
    imap.login(user, os.environ["EMAILOPS_IMAP_LIVE_PASSWORD"])
    return imap


def seed(user, subjects):
    imap = session(user)
    for subject in subjects:
        raw = (
            f"Message-ID: <ui-{uuid.uuid4().hex}@example.test>\r\n"
            f"Date: {email.utils.formatdate()}\r\n"
            "From: Sender Example <sender@example.test>\r\n"
            f"To: {user}\r\n"
            f"Subject: {subject}\r\n"
            "Content-Type: text/plain; charset=utf-8\r\n\r\n"
            f"Body of {subject}. Synthetic test message.\r\n"
        ).encode()
        imap.append("INBOX", None, None, raw)
    imap.logout()


def where(user, subject):
    imap = session(user)
    for folder in FOLDERS:
        imap.select(folder, readonly=True)
        _, data = imap.uid("SEARCH", None, "HEADER", "Subject", f'"{subject}"')
        uids = data[0].split() if data and data[0] else []
        if uids:
            _, flags = imap.uid("FETCH", uids[-1], "(FLAGS)")
            text = flags[0].decode() if flags and flags[0] else ""
            found = text[text.find("FLAGS (") + 7:].split(")")[0].split()
            imap.logout()
            return {"folder": folder, "flags": found}
    imap.logout()
    return {"folder": None, "flags": []}


if __name__ == "__main__":
    command, user, *rest = sys.argv[1:]
    if command == "seed":
        seed(user, rest)
    elif command == "where":
        print(json.dumps(where(user, rest[0])))
    else:
        sys.exit(f"unknown command {command}")
