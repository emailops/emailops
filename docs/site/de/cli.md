---
title: 'Kommandozeile (emailops-cli)'
description: 'Ihr Postfach vom Terminal aus skripten und automatisieren, mit stabiler JSON-Ausgabe für Skripte und Agenten.'
weight: 50
---

`emailops-cli` steuert dieselbe lokale Engine wie die Desktop-App — Ihre E-Mails, Ihre Konten,
Ihre lokale KI — vom Terminal aus. Es liest die Datenbank, die die App bereits synchronisiert
hat: keine separate Einrichtung, keine zweite Kopie Ihrer E-Mails.

Derzeit nur macOS.

## Installation

Laden Sie `EmailOps-CLI-macos.dmg` aus der
[neuesten Version](https://github.com/emailops/emailops/releases/latest) herunter, hängen Sie
es ein und legen Sie die Binärdatei in Ihren `PATH`:

```bash
hdiutil attach ~/Downloads/EmailOps-CLI-macos.dmg
cp /Volumes/EmailOps\ CLI/emailops-cli /usr/local/bin/emailops-cli
hdiutil detach /Volumes/EmailOps\ CLI

emailops-cli doctor    # prüft, ob Daten und Konten gefunden werden
```

Die Binärdatei ist universal (Apple Silicon + Intel), signiert und notarisiert — Gatekeeper
lässt sie ohne Rückfrage durch.

## Schnellstart

```bash
emailops-cli accounts                     # welche Konten verbunden sind
emailops-cli emails --limit 10            # die 10 neuesten E-Mails
emailops-cli search "Rechnung"            # Volltextsuche
emailops-cli chat "Was hat Acme zum Vertrag gesagt?"
emailops-cli                              # ohne Unterbefehl → interaktive REPL
```

In der REPL ist reiner Text ein Chat-Zug (Tokens erscheinen live), und Zeilen mit `/` am
Anfang entsprechen den Unterbefehlen: `/search`, `/account`, `/sync`, `/help`, `/quit`.

## Befehle

| Befehl | Zweck |
|---|---|
| `accounts` | Konfigurierte Konten auflisten |
| `emails [--limit N] [--mailbox inbox\|sent\|spam\|trash]` | Neueste E-Mails auflisten |
| `show <id>` | Eine E-Mail anzeigen (Kopfzeilen und Inhalt) |
| `search <Suchbegriff> [--limit N]` | Volltextsuche |
| `chat <Frage> [--trace]` | Eine Frage stellen; `--trace` ergänzt Routing- und Retrieval-Zeiten |
| `sync [Konto]` | Neue E-Mails herunterladen |
| `calendar [--days N] [--next] [--sync]` | Anstehende Termine (`--next` = nur der nächste) |
| `classify [--all]` | Neue — oder alle — E-Mails klassifizieren |
| `embed [--batch N]` | Such-Embeddings erzeugen |
| `doctor` | Schreibgeschützter Statusbericht (Datenbank, Konten, KI-Konfiguration) |

Globale Optionen funktionieren vor oder nach dem Unterbefehl: `--json`, `--quiet`,
`--account <id|E-Mail>`, `--model <Modell>`, `--data-dir <Verzeichnis>`.

Lesebefehle sind bei geöffneter App unbedenklich. Schreibintensive Befehle (`sync`,
`classify`, `embed`) führt man besser bei geschlossener App aus.

## Skripten mit `--json`

Mit `--json` gibt jeder Befehl genau einen Umschlag auf stdout aus — bei Erfolg wie bei
Fehlern in derselben Form — während Protokolle nach stderr gehen:

```jsonc
{ "ok": true,  "data": { /* Ergebnis */ }, "error": null }
{ "ok": false, "data": null, "error": { "code": "not_found", "message": "…", "params": {} } }
```

```bash
# Betreffzeilen der 20 neuesten E-Mails
emailops-cli emails --limit 20 --json | jq -r '.data[].subject'

# Nur der Antworttext einer Chat-Frage
emailops-cli chat "Fasse meine ungelesenen E-Mails zusammen" --json | jq -r '.data.answer'

# Absender und Betreff jedes Suchtreffers, als TSV
emailops-cli search "from:ana Rechnung" --json | jq -r '.data[] | [.sender, .subject] | @tsv'
```

Die Exit-Codes sind danach gruppiert, was zu tun wäre: `0` Erfolg, `2` ungültige Eingabe, `3`
nicht gefunden, `4` Authentifizierung, `5` Netzwerk/Synchronisierung, `6` KI, `130`
abgebrochen, `1` alles Übrige — Skripte können sich also am Code orientieren, statt Text zu
zerlegen.

Wenn Sie mehr als ein Konto haben, hinterlegen Sie einen Standard, statt `--account` zu
wiederholen:

```bash
emailops-cli config set default-account sie@beispiel.de
```
