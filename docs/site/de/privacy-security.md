---
title: 'Datenschutz und Sicherheit'
description: 'Wo Ihre E-Mails gespeichert werden, was Ihre Maschine verlässt, und die Schutzmechanismen gegen die E-Mails selbst.'
weight: 45
---

EmailOps ist um eine Regel herum gebaut: Ihre E-Mails bleiben auf Ihrer Maschine. Diese Seite
beschreibt konkret, was das bedeutet — wo Daten geschrieben werden, welche Netzwerkaufrufe es
gibt und welche Schutzfunktionen Sie einschalten können.

## Wo Ihre Daten gespeichert werden {#where-your-data-is-stored}

Alles liegt im Anwendungsdatenverzeichnis Ihres Betriebssystems:

| Plattform | Ort |
|---|---|
| macOS | `~/Library/Application Support/com.emailops.app` |
| Windows | `%APPDATA%\com.emailops.app` |
| Linux | `~/.local/share/com.emailops.app` |

Darin:

- **Eine SQLite-Datenbank** — Nachrichten, Threads, Kontakte, Kalendertermine,
  Klassifizierungs-Kennzeichnungen, Such-Embeddings und das KI-Gedächtnis. Das ist die einzige
  Kopie, die EmailOps führt.
- **Ein Ordner `models/`** — die KI-Modelle, die Sie heruntergeladen haben.

Zeigen Sie `EMAILOPS_DATA_DIR` vor dem Start woanders hin, um einen anderen Ort zu verwenden —
ein zweites Profil oder ein verschlüsseltes Volume.

**Zugangsdaten liegen nicht dort.** OAuth-Tokens und IMAP-Passwörter gehen in den
Anmeldeinformationsspeicher des Systems: macOS-Schlüsselbund, Windows-
Anmeldeinformationsverwaltung oder ein Secret-Service-Schlüsselbund unter Linux. Sie werden
nie in eine Konfigurationsdatei geschrieben und überstehen das Deinstallieren der App.

## Es gibt keinen EmailOps-Server

Es gibt kein Konto zum Anlegen, keine Registrierung und kein von uns betriebenes Backend — es
gibt also keinen Ort, an den Ihre E-Mails hochgeladen werden könnten, und nichts, das
kompromittiert werden könnte. Die App spricht genau mit diesen Zielen, die alle benennbar
sind:

| Ziel | Wann | Enthält Ihre E-Mails? |
|---|---|---|
| Ihr E-Mail-Anbieter (Gmail, Microsoft Graph, Ihr IMAP/SMTP-Server) | Bei jeder Synchronisierung und jedem Versand | Ja — es ist Ihr Postfach |
| Ihr Kalenderanbieter (Google, Outlook) | Kalendersynchronisierung, falls aktiviert | Nur Kalenderdaten |
| Hugging Face | Nur während des Downloads eines von Ihnen gewählten KI-Modells | Nein |
| OpenRouter | Nur wenn Sie den KI-Anbieter darauf umstellen | **Ja — Prompts enthalten E-Mail-Inhalte** |

Die letzte Zeile ist der einzige Weg, auf dem Ihre E-Mails zu einem Dritten gelangen können.
Sie ist standardmäßig aus und erfordert eine bewusste Änderung unter
**Einstellungen → KI-Backend & Modelle** sowie Ihren eigenen API-Schlüssel.

## Keine Telemetrie

Die App sammelt keine Nutzungsstatistiken, sendet keine Absturzberichte und ruft in
veröffentlichten Builds in keiner Form „nach Hause“. Es gibt kein Opt-out, weil es nichts
abzuwählen gibt. (Der Quellbaum enthält eine optionale OpenTelemetry-Tracing-Funktion für die
lokale Entwicklung; sie wird aus jedem Release-Build herauskompiliert.)

## Standardmäßig lokale KI

Das voreingestellte Backend führt Modelle im selben Prozess über eine eingebettete
llama.cpp-Laufzeit aus. Kein Daemon, kein lokaler Server, kein Netzwerk-Socket — das Modell
liest Ihre E-Mails aus dem Prozess, der sie ohnehin schon hat. Klassifizierung, Entwürfe,
Embeddings, Chat sowie Aufgaben- und Gedächtnis-Extraktion laufen alle dort.

Der Wechsel zu Ollama hält die Inferenz ebenfalls lokal, nur in einem eigenen Prozess auf
Ihrer Maschine. Nur OpenRouter sendet Inhalte vom Gerät weg. Siehe
[Backend wählen](../ai-features/#choosing-a-backend).

## Schutz vor den E-Mails selbst

E-Mail ist eine Angriffsfläche. Die Schutzmechanismen auf Client-Seite:

- **Blockieren entfernter Inhalte** — externe Bilder, Tracking-Pixel und andere entfernte
  Ressourcen werden blockiert, bis Sie sie erlauben. Ein Hinweisbalken je E-Mail lädt sie
  einmalig, oder Sie vertrauen einem bestimmten Absender dauerhaft. Das verhindert, dass
  Absender erfahren, wann und wie oft Sie eine Nachricht geöffnet haben.
- **Junk- und Massenbewertung** — jede Nachricht wird lokal auf Spam und unerwünschte
  Massen-E-Mails bewertet. Ihre Korrekturen („Junk“ / „kein Junk“) trainieren sie. Markierte
  Post wird abgeschwächt oder ausgeblendet, aber nie auf dem Server gelöscht oder verschoben,
  außer Sie bestätigen es ausdrücklich.
- **Warnungen vor Identitätsmissbrauch** — eine optionale Prüfung, die Nachrichten markiert,
  die scheinbar von jemand anderem stammen. Standardmäßig aus, denn es ist die einzige
  Prüfung, die einem Absender Betrug unterstellt, und sie hat die dünnste Beweislage.
- **Bereinigtes Rendering** — das HTML der Nachrichten wird vor der Anzeige von Skripten,
  Event-Handlern und eingebetteten Objekten befreit, und zwar auf beiden Seiten der App.
  Anhänge werden nie eigenmächtig geöffnet.

## Die App sperren

Legen Sie unter **Einstellungen → Datenschutz & Sicherheit** ein **Hauptpasswort** fest, dann
bleibt EmailOps beim Start gesperrt, bis Sie es eingeben. Es gibt keinen Wiederherstellungsweg
— wenn Sie es vergessen, installieren Sie neu gegen ein frisches Datenverzeichnis und
synchronisieren erneut von Ihrem Anbieter.

Zur Klarheit, was das leistet: Es sperrt die Anwendung, es verschlüsselt die Datenbank
**nicht**. Wer Zugriff auf Ihr entsperrtes Benutzerkonto und das Datenverzeichnis hat, kann
die SQLite-Datei direkt lesen. Wenn das zu Ihrem Bedrohungsmodell gehört, nutzen Sie
Festplattenverschlüsselung — FileVault unter macOS, BitLocker unter Windows, LUKS unter
Linux — das ist das richtige Werkzeug dafür.

## All das überprüfen

EmailOps steht unter Apache-2.0 und wird offen entwickelt. Die Aussagen auf dieser Seite sind
anhand des Quellcodes auf
[github.com/emailops/emailops](https://github.com/emailops/emailops) überprüfbar, und das
Netzwerkverhalten ebenso — betreiben Sie die App hinter einem Proxy oder mit `tcpdump` und
vergleichen Sie mit der Tabelle oben. Falls etwas nicht übereinstimmt,
[eröffnen Sie bitte ein Issue](https://github.com/emailops/emailops/issues).
