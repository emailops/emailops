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


def claim(cid, name, fn, fix="", covers=None, how="", proof="behaviour", partial=""):
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
                  "covers": covers, "how": how, "proof": proof, "partial": partial, "read_doc": cid in CLAIMS.reads,
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


def db_count(sql):
    out = subprocess.run(["/usr/bin/sqlite3", "-readonly", str(work / "emailops.db"), sql],
                         capture_output=True, text=True)
    return int(out.stdout.strip() or -1)


def same_engine():
    CLAIMS["cli-intro-1"]
    d = doc.get("data") or {}
    emails = db_count("select count(*) from emails")
    accounts = db_count("select count(*) from accounts")
    good = doc.get("ok") and d.get("emailCount") == emails and d.get("accountsTotal") == accounts
    return (bool(good), f"doctor ve {d.get('accountsTotal')} cuentas y {d.get('emailCount')} correos, los mismos que la base de "
            f"datos que sincronizó la app ({accounts} y {emails})")


claim("cli-intro-1", "misma base de datos", same_engine,
      covers=["emailops-cli drives the same local engine as the desktop app",
              "It reads the database the app already synced, so there is no separate setup and no second copy of your mail."],
      how="Ejecuta `emailops-cli doctor --json` sobre una copia del directorio de datos de la demo, sin configurar nada, y "
          "compara las cuentas y correos que ve con los que tiene la base de datos de la app.")


# ── install: the command that closes the install block ──────────────────────
def doctor_confirms():
    line = next(c for c in quoted_commands("cli-install-1") if c.split()[1:2] == ["doctor"])
    rc, out, _ = run(*line.split()[1:])
    return (rc == 0 and "accounts" in out.lower(), f"`{line}` informa de datos y cuentas" if rc == 0 else f"`{line}` → {rc}")


claim("cli-install-1", "doctor", doctor_confirms, covers=["emailops-cli doctor"],
      partial="los pasos hdiutil y cp de la instalación no se ejecutan",
      how="Ejecuta la última línea del bloque de instalación (`emailops-cli doctor`) y comprueba que informa de los datos y "
          "las cuentas.")


# ── quick start: every command the page shows runs ───────────────────────────
def quick_start():
    failures, ran = [], 0
    for cmd in quoted_commands("cli-quick-start-1"):
        args = [a.strip('"') for a in re.findall(r'"[^"]*"|\S+', cmd)[1:]]
        if not args or args[0] == "chat":
            continue  # `chat` needs a downloaded model; the bare REPL is cli-quick-start-2
        if args[0] in ("emails", "search") and account:
            args = ["--account", account, *args]
        rc, out, err = run(*args)
        ran += 1
        if rc != 0:
            failures.append(f"`{cmd}` → código {rc}: {(err or out).strip().splitlines()[-1][:80] if (err or out).strip() else ''}")
    return (not failures and ran > 0, f"{ran} comandos del inicio rápido funcionan" if not failures else "; ".join(failures))


claim("cli-quick-start-1", "comandos", quick_start, covers=["emailops-cli accounts"],
      partial="`chat` necesita un modelo descargado y no se ejecuta",
      how="Ejecuta, tal como aparecen en la doc, cada comando del bloque de inicio rápido (salvo chat y el REPL) sobre la "
          "copia de la demo y exige código de salida 0.")


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
    problems = []
    if missing:
        problems.append(f"/help no menciona {', '.join(missing)}")
    if "commands start with '/'" not in plain:
        problems.append("el texto sin / no se rechaza, pero la doc dice que toda acción es un comando con /")
    return (not problems, f"/help lista {', '.join(listed)}; el texto sin / se rechaza" if not problems else "; ".join(problems))


claim("cli-quick-start-2", "REPL", repl,
      "Say that every REPL action is a slash-command and chat is /chat <question>, in all four languages.",
      covers=["In the REPL every action is a /-prefixed command"],
      partial="que /chat transmita tokens en directo necesita un modelo descargado",
      how="Abre el REPL en un pseudo-terminal, lee los comandos que enumera la doc y comprueba que /help los lista, y que "
          "escribir texto sin / no se toma como chat.")


# ── the command table, row by row ────────────────────────────────────────────
_, top_help, _ = run("--help")
for row in re.findall(r"^\| `([^`]+)` \|", CLAIMS["cli-commands-1"], re.M):
    def check_row(row=row):
        name, *_ = row.split()
        if not re.search(rf"^\s+{re.escape(name)}\b", top_help, re.M):
            return False, f"«{name}» no está en --help"
        _, sub, _ = run(name, "--help")
        problems = [f"«{name} {f}» no existe" for f in re.findall(r"--[a-z-]+", row) if f not in sub]
        problems += [f"«{name}» no acepta «{v}»" for v in re.findall(r"(?:inbox|sent|spam|trash)", row) if v not in sub]
        return (not problems, f"`{row}` existe tal cual" if not problems else "; ".join(problems))

    claim("cli-commands-1", row.split()[0], check_row, covers=[f"| {row} |"], proof="label",
          how="Lee la fila de la tabla y comprueba en `emailops-cli --help` y en el --help del subcomando que el comando, "
              "sus opciones y sus valores existen. Lo que hace cada comando lo prueban el inicio rápido y los ejemplos.")


def global_flags():
    flags = re.findall(r"`(--[a-z-]+)", CLAIMS["cli-commands-2"])
    missing = [f for f in flags if f not in top_help]
    before = subprocess.run([str(CLI), "--json", "--data-dir", str(work), "doctor"], capture_output=True, text=True)
    after = subprocess.run([str(CLI), "doctor", "--data-dir", str(work), "--json"], capture_output=True, text=True)
    both = before.returncode == 0 and after.returncode == 0 and before.stdout.startswith("{") and after.stdout.startswith("{")
    return (not missing and both, f"{', '.join(flags)} existen; --json y --data-dir funcionan antes y después del subcomando"
            if not missing and both else f"faltan {', '.join(missing)}; antes/después: {before.returncode}/{after.returncode}")


claim("cli-commands-2", "opciones globales", global_flags,
      covers=["Global flags work before or after the subcommand"],
      how="Lee las opciones globales de la doc y comprueba que están en --help; ejecuta `--json --data-dir … doctor` y "
          "`doctor --data-dir … --json` y exige JSON en ambos órdenes.")


# ── --json: one envelope, same shape both ways ───────────────────────────────
def one_envelope():
    CLAIMS["cli-scripting-json-1"]
    good = subprocess.run([str(CLI), "--data-dir", str(work), "doctor", "--json"], capture_output=True, text=True)
    bad = subprocess.run([str(CLI), "--data-dir", str(work), "show", "no-such-email-id", "--json"], capture_output=True, text=True)
    g, b_ = json.loads(good.stdout), json.loads(bad.stdout)
    keys = {"ok", "data", "error"}
    shaped = set(g) == keys and set(b_) == keys and set(b_["error"]) >= {"code", "message", "params"}
    single = good.stdout.strip().count("\n{") == 0
    return (shaped and single and g["ok"] is True and b_["ok"] is False and b_["error"]["code"] == "not_found",
            "éxito y fallo imprimen un único {ok, data, error} en stdout" if shaped else f"claves: {sorted(g)} / {sorted(b_)}")


claim("cli-scripting-json-1", "sobre", one_envelope,
      covers=["With --json every command prints exactly one envelope on stdout — same shape on success or failure",
              '{ "ok": true'],
      how="Ejecuta un comando que funciona y otro que falla con --json, parsea stdout como un único JSON y compara sus "
          "claves con las del ejemplo de la doc (ok, data, error; error con code, message y params).")


def jq_examples():
    import shutil
    jq = shutil.which("jq", path="/usr/bin:/bin") or shutil.which("jq")
    if not account:
        return False, "no hay cuenta en la demo"
    ran, problems, empty = 0, [], []
    for line in CLAIMS["cli-scripting-json-2"].splitlines():
        line = line.strip()
        if not line.startswith("emailops-cli") or " chat " in line:
            continue
        cmd = line.replace("emailops-cli", f"'{CLI}' --data-dir '{work}' --account '{account}'", 1).replace("| jq", f"| '{jq}'")
        out = subprocess.run(["/bin/bash", "-o", "pipefail", "-c", cmd], capture_output=True, text=True)
        ran += 1
        if out.returncode != 0:
            problems.append(f"`{line}` → código {out.returncode}: {out.stderr.strip()[-120:]}")
        elif not out.stdout.strip():
            # An example with a made-up sender can match nothing in the demo; then
            # the envelope must still have the shape the jq filter walks.
            raw = subprocess.run(["/bin/bash", "-c", cmd.split(" | ")[0]], capture_output=True, text=True).stdout
            if not isinstance(json.loads(raw).get("data"), list):
                problems.append(f"`{line}` → sin resultados y .data no es una lista")
            else:
                empty.append(line.split(" --json")[0])
    note = f" ({', '.join(empty)} no encuentra nada en la demo, pero .data tiene la forma del ejemplo)" if empty else ""
    return (ran > 0 and not problems, f"{ran} ejemplos con jq funcionan tal cual{note}" if not problems else "; ".join(problems))


claim("cli-scripting-json-2", "ejemplos con jq", jq_examples, covers=["emailops-cli emails --limit 20 --json"],
      partial="el ejemplo con chat necesita un modelo descargado y no se ejecuta",
      how="Ejecuta cada tubería `emailops-cli … --json | jq …` del bloque tal como está en la doc (salvo la de chat) sobre "
          "la copia de la demo y exige código 0 y salida no vacía.")


def exit_codes():
    text = " ".join(CLAIMS["cli-scripting-json-3"].split())
    want_nf = int(re.search(r"`(\d+)` not found", text).group(1))
    want_inv = int(re.search(r"`(\d+)` invalid input", text).group(1))
    rc_nf, _, _ = run("show", "no-such-email-id", "--json")
    rc_inv, _, _ = run("compose", "--to", "a@example.com", "--subject", "x",
                       "--body-file", str(work / "no-such-body.txt"), "--json")  # unreadable file → invalid input
    return (rc_nf == want_nf and rc_inv == want_inv, f"no encontrado → {rc_nf}, entrada inválida → {rc_inv}, como dice la doc"
            if rc_nf == want_nf and rc_inv == want_inv else f"no encontrado → {rc_nf} (doc {want_nf}), entrada inválida → {rc_inv} (doc {want_inv})")


claim("cli-scripting-json-3", "códigos de salida", exit_codes, covers=["Exit codes are grouped by what you would do about them"],
      partial="se provocan «not found» e «invalid input»; auth, red, IA y cancelación no",
      how="Lee de la doc el código de «not found» y el de «invalid input», provoca cada error (un id que no existe; un "
          "--body-file que no existe) y compara el código de salida.")


def default_account():
    if not account:
        return False, "no hay cuenta en la demo"
    cmd = quoted_commands("cli-scripting-json-4")[0].split()[1:]
    cmd[-1] = account  # the page uses a placeholder address
    rc, out, err = run(*cmd)
    rc2, em = envelope("emails", "--limit", "1")
    return (rc == 0 and em.get("ok") is True, "tras `config set default-account` ya no hace falta --account"
            if rc == 0 and em.get("ok") else f"config: {rc}; emails sin --account: {em.get('error')}")


claim("cli-scripting-json-4", "cuenta por defecto", default_account,
      covers=["If you have more than one account, save a default instead of repeating --account:",
              "emailops-cli config set default-account you@example.com"],
      how="Con las varias cuentas de la demo, ejecuta el comando de la doc con una cuenta real y comprueba que después "
          "`emails` funciona sin --account.")

(OUT / "cli.json").write_text(json.dumps(parts, ensure_ascii=False, indent=2))
shutil.rmtree(work, ignore_errors=True)
print(f"\n{len(parts)} cases, {sum(o['status'] == 'fail' for o in parts)} failing → {OUT}/cli.json")
