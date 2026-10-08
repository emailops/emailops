"""Make a scratch copy of the demo data dir and add synthetic emails to it.

Data a teaser needs and the demo lacks (an email in another language for the
translation beat, say) goes here, never into the repo's demo DB.

    uv run --no-project python .claude/skills/record-emailops-teaser/scripts/scratch_demo.py \\
        <scratch_dir> [emails.json]

emails.json: a list of {"account": "work"|"personal", "sender_name", "sender_email",
"subject", "body", "minutes_ago": 30, "category": "primary"}. Everything in it
must be synthetic: invented names, `.example` domains.

Then launch on the copy:
    VERIFY_DATA_DIR=<scratch_dir> .claude/skills/verify-emailops/scripts/verify.sh launch
For a retake, run this again on a fresh dir rather than undoing state in the app.
"""
import json
import sqlite3
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO / "scripts"))
import generate_demo_db as g  # noqa: E402

DEMO = REPO / ".emailops-demo-data"
ACCOUNTS = {"work": g.ACCOUNT_WORK, "personal": g.ACCOUNT_PERSONAL}


def main():
    dest = Path(sys.argv[1]).resolve()
    if dest == DEMO.resolve() or "com.emailops.app" in str(dest):
        sys.exit("refusing: the scratch dir must not be the demo dir or the production data dir")
    dest.mkdir(parents=True, exist_ok=True)
    db = dest / "emailops.db"
    if db.exists():
        sys.exit(f"{db} exists: use a fresh directory for a clean take")
    src = sqlite3.connect(f"file:{DEMO / 'emailops.db'}?immutable=1", uri=True)
    dst = sqlite3.connect(db)
    src.backup(dst)
    src.close()
    (dest / "models").symlink_to(DEMO / "models")
    if len(sys.argv) > 2:
        for e in json.loads(Path(sys.argv[2]).read_text()):
            eid = g.insert_email(
                dst, account=ACCOUNTS[e.get("account", "work")], sender_name=e["sender_name"],
                sender_email=e["sender_email"], subject=e["subject"], body=e["body"],
                timestamp=int(time.time()) - 60 * int(e.get("minutes_ago", 30)), is_read=False,
                mailbox="inbox", category=e.get("category", "primary"))
            print("inserted", eid, e["subject"])
        dst.commit()
    dst.close()
    print(f"scratch demo ready: {dest}")


if __name__ == "__main__":
    main()
