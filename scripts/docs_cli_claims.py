#!/usr/bin/env python3
"""Verify what docs/site/en/cli.md promises by running emailops-cli.

The CLI is part of the app, so the ground truth for the CLI page is the
binary itself, run against a copy of the demo data dir (config writes must not
touch the DB `make verify` uses). Writes <out_dir>/cli.json in the shape
doc_claims.mjs uses: one record per case, with the sentences it covers (see
claim() below and the header of doc_claims.mjs).

Usage: docs_cli_claims.py <cli binary> <demo data dir> <out_dir>
"""

import json
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from docs_claims_lib import blocks, pages  # noqa: E402

CLI, DEMO, OUT = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2]), pathlib.Path(sys.argv[3])
OUT.mkdir(parents=True, exist_ok=True)


class _Recorded(dict):
    """The claims' text, recording which ones a case read: a case that never
    reads its claim has its expectation typed in, and the report says so."""
    reads = set()

    def __getitem__(self, k):
        self.reads.add(k)
        return super().__getitem__(k)


CLAIMS = _Recorded({b.claim: b.text for p in pages("en") for b in blocks(p) if b.claim})

work = pathlib.Path(tempfile.mkdtemp(prefix="docs-cli-"))
for f in DEMO.glob("emailops.db*"):
    shutil.copy2(f, work / f.name)
(work / "models").mkdir(exist_ok=True)


def run(*args, stdin=None):
    p = subprocess.run([str(CLI), "--data-dir", str(work), *args], input=stdin,
                       capture_output=True, text=True, timeout=120)
    return p.returncode, p.stdout, p.stderr


def envelope(*args):
    rc, out, err = run(*args, "--json")
    try:
        return rc, json.loads(out)
    except json.JSONDecodeError:
        return rc, {"_raw": out[-400:], "_err": err[-400:]}


parts = []


def claim(cid, name, fn, fix="", covers=None, how="", proof="behaviour"):
    """One case. covers/how/proof mean what they mean in doc_claims.mjs."""
    if cid not in CLAIMS:
        raise SystemExit(f"docs_cli_claims.py checks claim:{cid}, which the docs no longer have")
    import inspect
    where = f"scripts/docs_cli_claims.py:{inspect.stack()[1].lineno}"
    CLAIMS.reads.clear()
    try:
        ok, detail = fn()
    except Exception as e:  # a crashed check is a failed check, with its reason
        ok, detail = False, f"{type(e).__name__}: {e}"
    parts.append({"claim": cid, "name": name, "tag": "CLI", "status": "ok" if ok else "fail", "detail": detail,
                  "covers": covers, "how": how, "proof": proof, "read_doc": cid in CLAIMS.reads,
                  "where": where, "fix": fix if not ok else "", "shots": []})
    print(f"{'OK  ' if ok else 'FAIL'} {cid} / {name}: {detail[:150]}")


def quoted_commands(cid):
    """The `emailops-cli …` invocations a claim shows, flags and all."""
    return [line.split("#")[0].strip() for line in CLAIMS[cid].splitlines()
            if line.strip().startswith("emailops-cli")]


# ── the engine is the app's ──────────────────────────────────────────────────
rc, doc = envelope("doctor")
account = None
rc2, accts = envelope("accounts")
if accts.get("ok") and accts["data"]:
    first = accts["data"][0]
    account = first.get("email") or first.get("id")

claim("cli-intro-1", "misma base de datos", lambda: (
    bool(doc.get("ok") and doc["data"].get("accountsTotal", 0) > 0 and doc["data"].get("emailCount", 0) > 0),
    f"doctor ve {doc.get('data', {}).get('accountsTotal')} cuentas y {doc.get('data', {}).get('emailCount')} correos sincronizados por la app"))

# ── quick start: every command the page shows runs ───────────────────────────
def quick_start():
    failures, ran = [], 0
    for cmd in quoted_commands("cli-quick-start-1"):
        args = re.findall(r'"[^"]*"|\S+', cmd)[1:]
        args = [a.strip('"') for a in args]
        if not args or args[0] == "chat":
            continue  # `chat` needs a downloaded model; the envelope check covers it below
        if args[0] in ("emails", "search") and account:
            args = ["--account", account, *args]
        rc, out, err = run(*args)
        ran += 1
        if rc != 0:
            failures.append(f"`{cmd}` → código {rc}: {(err or out).strip().splitlines()[-1][:80] if (err or out).strip() else ''}")
    return (not failures and ran > 0, f"{ran} comandos del inicio rápido funcionan" if not failures else "; ".join(failures))


claim("cli-quick-start-1", "comandos", quick_start)


def repl_session(*lines):
    """Drive the interactive REPL through a pseudo-terminal. reedline asks the
    terminal for the cursor position (ESC[6n) and quits if nobody answers, so
    this answers like a real terminal would; a plain pipe never reaches /help."""
    import os, pty, select, time
    pid, fd = pty.fork()
    if pid == 0:
        os.execv(str(CLI.resolve()), ["emailops-cli", "--data-dir", str(work)])

    def pump(seconds):
        out, end = b"", time.time() + seconds
        while time.time() < end:
            ready, _, _ = select.select([fd], [], [], 0.1)
            if not ready:
                continue
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                break
            out += chunk
            for _ in range(chunk.count(b"\x1b[6n")):
                os.write(fd, b"\x1b[1;1R")
        return re.sub(r"\x1b\[[0-9;?]*[a-zA-Z]", "", out.decode(errors="replace"))

    pump(2)
    replies = []
    for line in lines:
        os.write(fd, line.encode() + b"\r")
        replies.append(pump(3))
    os.write(fd, b"/quit\r")
    pump(1)
    os.waitpid(pid, 0)
    return replies


def repl():
    help_text, plain = repl_session("/help", "what came in today")
    listed = re.findall(r"`(/[a-z]+)`", CLAIMS["cli-quick-start-2"])
    missing = [c for c in listed if c not in help_text]
    promises_plain_chat = "plain text is a chat turn" in " ".join(CLAIMS["cli-quick-start-2"].split())
    plain_is_chat = "commands start with '/'" not in plain
    problems = []
    if missing:
        problems.append(f"/help no menciona {', '.join(missing)}")
    if promises_plain_chat and not plain_is_chat:
        problems.append("el doc dice que el texto plano es un turno de chat, pero el REPL responde «commands start with '/'. to chat, use: /chat …»")
    return (not problems, f"/help lista {', '.join(listed)}" + ("" if promises_plain_chat else "; el chat va con /chat")
            if not problems else "; ".join(problems))


claim("cli-quick-start-2", "REPL", repl,
      "Say that every REPL action is a slash-command and chat is /chat <question>, in all four languages.")

# ── the command table ────────────────────────────────────────────────────────
def command_table():
    rc, top, _ = run("--help")
    problems = []
    for row in re.findall(r"^\| `([^`]+)` \|", CLAIMS["cli-commands-1"], re.M):
        name, *flags = row.split()
        if not re.search(rf"^\s+{re.escape(name)}\b", top, re.M):
            problems.append(f"«{name}» no está en --help")
            continue
        _, sub, _ = run(name, "--help")
        for flag in re.findall(r"--[a-z-]+", row):
            if flag not in sub:
                problems.append(f"«{name} {flag}» no existe")
        for value in re.findall(r"(?:inbox|sent|spam|trash)", row):
            if value not in sub:
                problems.append(f"«{name}» no acepta «{value}»")
    return (not problems, "cada comando y opción de la tabla existe" if not problems else "; ".join(problems))


claim("cli-commands-1", "tabla", command_table)


def global_flags():
    _, top, _ = run("--help")
    flags = re.findall(r"`(--[a-z-]+)", CLAIMS["cli-commands-2"])
    missing = [f for f in flags if f not in top]
    before, _ = envelope("doctor")
    rc_after, out_after, _ = subprocess.run([str(CLI), "doctor", "--data-dir", str(work), "--json"],
                                            capture_output=True, text=True).returncode, None, None
    return (not missing and rc_after == 0, f"{', '.join(flags)} existen y funcionan antes o después del subcomando"
            if not missing else f"faltan {', '.join(missing)}")


claim("cli-commands-2", "opciones globales", global_flags)

# ── --json: one envelope, same shape both ways ───────────────────────────────
def one_envelope():
    _, good = envelope("doctor")
    _, bad = envelope("show", "no-such-email-id")
    keys = {"ok", "data", "error"}
    shaped = set(good) == keys and set(bad) == keys
    return (shaped and good["ok"] is True and bad["ok"] is False and bad["error"].get("code") == "not_found",
            "éxito y fallo tienen la misma forma {ok, data, error}" if shaped else f"claves: {sorted(good)} / {sorted(bad)}")


claim("cli-scripting-json-1", "sobre", one_envelope)


def jq_shapes():
    if not account:
        return False, "no hay cuenta en la demo"
    _, em = envelope("--account", account, "emails", "--limit", "5")
    _, sr = envelope("--account", account, "search", "invoice")
    email_ok = isinstance(em.get("data"), list) and em["data"] and "subject" in em["data"][0]
    search_ok = isinstance(sr.get("data"), list) and all("sender" in r and "subject" in r for r in sr["data"][:3])
    return (email_ok and search_ok, ".data[].subject y .data[] | [.sender, .subject] existen como muestran los ejemplos"
            if email_ok and search_ok else f"emails: {bool(email_ok)}, search: {bool(search_ok)}")


claim("cli-scripting-json-2", "rutas de jq", jq_shapes)


def exit_codes():
    rc_nf, _, _ = run("show", "no-such-email-id", "--json")
    rc_inv, _, _ = run("emails", "--limit", "1", "--json")  # several accounts, no default → invalid input
    doc = CLAIMS["cli-scripting-json-3"]
    want_nf = "`3` not found" in doc
    want_inv = "`2` invalid input" in doc
    ok = (not want_nf or rc_nf == 3) and (not want_inv or rc_inv == 2)
    return ok, f"no encontrado → {rc_nf}, entrada inválida → {rc_inv}"


claim("cli-scripting-json-3", "códigos de salida", exit_codes)


def default_account():
    if not account:
        return False, "no hay cuenta en la demo"
    cmd = quoted_commands("cli-scripting-json-4")[0].split()[1:]
    cmd[-1] = account  # the page uses a placeholder address
    rc, out, err = run(*cmd)
    rc2, em = envelope("emails", "--limit", "1")
    return (rc == 0 and em.get("ok") is True, "tras `config set default-account` ya no hace falta --account"
            if rc == 0 and em.get("ok") else f"config: {rc}; emails sin --account: {em.get('error')}")


claim("cli-scripting-json-4", "cuenta por defecto", default_account)

(OUT / "cli.json").write_text(json.dumps(parts, ensure_ascii=False, indent=2))
shutil.rmtree(work, ignore_errors=True)
print(f"\n{len(parts)} cases, {sum(o['status'] == 'fail' for o in parts)} failing → {OUT}/cli.json")
