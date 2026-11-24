---
title: 'Datenschutz und Sicherheit'
description: 'Wo Ihre E-Mails gespeichert werden, was Ihre Maschine verlässt, und die Schutzmechanismen gegen die E-Mails selbst.'
weight: 45
nav:
  where-your-data-is-stored: settings/data
  protection-from-the-mail-itself: settings/privacy
  locking-the-app: settings/privacy
---

<!-- claim:priv-intro-1 -->
EmailOps ist um eine Regel herum gebaut: Ihre E-Mails bleiben auf Ihrer Maschine. Diese Seite
beschreibt konkret, was das bedeutet — wo Daten geschrieben werden, welche Netzwerkaufrufe es
gibt und welche Schutzfunktionen Sie einschalten können.

## Wo Ihre Daten gespeichert werden {#where-your-data-is-stored}

<!-- claim:priv-where-data-1 -->
Alles liegt im Anwendungsdatenverzeichnis Ihres Betriebssystems:

| Plattform | Ort |
|---|---|
| macOS | `~/Library/Application Support/com.emailops.app` |
| Windows | `%APPDATA%\com.emailops.app` |
| Linux | `~/.local/share/com.emailops.app` |

<!-- claim:priv-where-data-2 -->
Darin:

- **Eine SQLite-Datenbank** — Nachrichten, Threads, Kontakte, Kalendertermine,
  Klassifizierungs-Kennzeichnungen, Such-Embeddings und das KI-Gedächtnis. <!-- claim:priv-where-data-3 -->
- **Ein Ordner `models/`** — die KI-Modelle, die Sie heruntergeladen haben. <!-- claim:priv-where-data-4 -->
- **Ein Ordner `backups/`** — vollständige Kopien der Datenbank, die EmailOps anlegt, solange es
  geöffnet ist: standardmäßig eine pro Tag um 03:00, nur die neueste wird aufbewahrt. Time Machine
  überspringt diesen Ordner. Unter **Einstellungen → Daten & Sicherungen → Sicherungen**
  schalten Sie sie aus, stellen auf wöchentlich um, behalten bis zu drei oder löschen sie. <!-- claim:priv-where-data-8 -->
- **Ein Ordner `local-backups/`** — kleine Kopien dessen, was es nur auf diesem Computer gibt:
  Einstellungen, Konten ohne ihre Passwörter, Regeln, Gedächtnis, Aufgaben, Linsen und Chats.
  E-Mails sind nicht enthalten. Standardmäßig eine pro Tag, die neuesten sieben werden
  aufbewahrt; im selben Abschnitt stellen Sie eine wieder her. <!-- claim:priv-where-data-9 -->

<!-- claim:priv-where-data-5 -->
Um ihn woanders aufzubewahren — auf einem externen Laufwerk oder einem verschlüsselten Volume —
nutzen Sie **Einstellungen → Daten & Sicherungen → Speicherort der Daten** oder den Link unten im
ersten Schritt der Einrichtung. EmailOps kopiert alles dorthin, startet neu und lässt den alten
Ordner liegen, damit Sie ihn selbst löschen. Ist das Laufwerk beim Start nicht angeschlossen,
meldet EmailOps das und beendet sich, statt ein leeres Postfach zu öffnen. Ein mit **Vorhandene Datei verwenden…** hinzugefügtes
Modell wird nicht kopiert: Der neue Ordner verweist auf dieselbe Datei. EmailOps verschlüsselt die Daten nicht, wählen Sie also ein
verschlüsseltes Laufwerk; den Ordner, den es dort anlegt, kann nur Ihr Benutzer lesen. Eine vor dem Start
gesetzte `EMAILOPS_DATA_DIR` (für ein zweites Profil) hat Vorrang vor dieser Wahl.

<!-- claim:priv-where-data-6 -->
**Zugangsdaten liegen nicht dort.** OAuth-Tokens und IMAP-Passwörter gehen in den
Anmeldeinformationsspeicher des Systems: macOS-Schlüsselbund, Windows-
Anmeldeinformationsverwaltung oder ein Secret-Service-Schlüsselbund unter Linux. Sie werden
nie in eine Konfigurationsdatei geschrieben und überstehen das Deinstallieren der App.

<!-- claim:priv-where-data-7 -->
Wenn Sie ein Gmail-Konto entfernen, wird auch der Zugriff von EmailOps bei Google widerrufen;
schlägt das fehl, sagt das Protokoll, wo Sie ihn von Hand entfernen. Microsoft lässt eine App
ihren eigenen Zugriff nicht widerrufen, daher verweist das Entfernen eines Outlook-Kontos auf
die Microsoft-Seiten, auf denen Sie EmailOps selbst entfernen.

## Es gibt keinen EmailOps-Server

<!-- claim:priv-there-no-1 -->
Es gibt kein Konto zum Anlegen, keine Registrierung und kein von uns betriebenes Backend — es
gibt also keinen Ort, an den Ihre E-Mails hochgeladen werden könnten, und nichts, das
kompromittiert werden könnte. Die App spricht genau mit diesen Zielen, die alle benennbar
sind:

| Ziel | Wann | Enthält Ihre E-Mails? |
|---|---|---|
| Ihr E-Mail-Anbieter (Gmail, Microsoft Graph, Ihr IMAP/SMTP-Server) | Bei jeder Synchronisierung und jedem Versand | Ja — es ist Ihr Postfach |
| Ihr Kalenderanbieter (Google, Outlook) | Kalendersynchronisierung, falls aktiviert | Nur Kalenderdaten |
| Hugging Face | Nur während des Downloads eines von Ihnen gewählten KI-Modells | Nein |
| getemailops.com | Einmal am Tag, um die neueste Versionsnummer zu lesen | Nein |
| Ihr OpenAI-kompatibler Server | Nur wenn Sie den KI-Anbieter darauf umstellen | **Ja — Prompts enthalten E-Mail-Inhalte** |
| OpenRouter | Nur wenn Sie den KI-Anbieter darauf umstellen | **Ja — Prompts enthalten E-Mail-Inhalte** |

<!-- claim:priv-there-no-2 -->
Abgesehen von einem Server, den Sie selbst betreiben, ist die letzte Zeile der einzige Weg, auf dem Ihre E-Mails zu einem Dritten gelangen können.
Sie ist standardmäßig aus und erfordert eine bewusste Änderung unter
**Einstellungen → KI: Backend & Modelle** sowie Ihren eigenen API-Schlüssel.

<!-- claim:priv-there-no-5 -->
Ein OpenAI-kompatibler Server auf diesem Computer behält Ihre Post dort. Einer auf einem anderen
Rechner muss https verwenden, und beim Speichern bestätigen Sie für seinen Host, dass Ihre Post
dorthin gesendet wird und dass EmailOps nicht steuern kann, was er aufbewahrt; siehe
[ein OpenAI-kompatibler Server](../ai-features/#openai-compatible-server).

<!-- claim:priv-there-no-3 -->
Wenn Sie ihn aktivieren, weist jede Anfrage OpenRouter an, nur Anbieter zu verwenden, die das
Empfangene weder speichern noch damit trainieren; ein Modell, das kein solcher Anbieter
bereitstellt, wird mit einem Fehler abgelehnt und nie trotzdem gesendet. **Nur ohne
Datenspeicherung** im selben Bereich beschränkt das auf Anbieter, die nach der Antwort nichts
aufbewahren.

<!-- claim:priv-there-no-4 -->
Dasselbe gilt für die Suche. OpenRouter bettet Ihre E-Mails nur ein, wenn Sie in diesem Bereich
ein Embedding-Modell gewählt haben: Der Text jeder indizierten E-Mail sowie jeder Suche und
Chat-Frage geht dann nach denselben Anbieterregeln hinaus. Ohne Modell wird zum Indizieren
nichts gesendet und es wird nur nach Stichwörtern gesucht.

## Was EmailOps in Ihrem Postfach verändert

<!-- claim:priv-what-emailops-1 -->
Das meiste, was EmailOps tut, ist rein lesend: Es lädt Ihre E-Mails herunter und hält eine
lokale Kopie. Einige Aktionen greifen bewusst auf das Konto zurück, damit das, was Sie hier tun,
auch überall sonst zu sehen ist:

| Aktion | Wirkung auf das Konto |
|---|---|
| Eine Nachricht als gelesen oder ungelesen markieren | Dieselbe Nachricht wird im Konto als gelesen markiert |
| Eine Nachricht löschen | Die Nachricht wandert in den **Papierkorb** des Kontos (Gelöschte Elemente bei Outlook) und bleibt dort wiederherstellbar, bis der Anbieter ihn leert – bei Gmail 30 Tage |
| **Endgültig löschen**, **Papierkorb leeren** oder **Spam leeren**, bei einem IMAP- oder Outlook-Konto | Die Nachrichten werden aus dem Papierkorb oder Spam des Kontos endgültig gelöscht; kein anderer Ordner wird angefasst |
| Eine Nachricht in einen anderen Ordner verschieben, oder **Als Spam bestätigen** | Die Nachricht wird auch im Konto verschoben — Als Spam bestätigen legt sie im Spam-Ordner des Anbieters ab |
| **Kein Spam — in den Posteingang verschieben**, oder **Kein Spam** bei einer Nachricht in **Spam** | Die Nachricht wandert zurück in den Posteingang des Kontos |
| Einen Ordner anlegen, umbenennen oder löschen | Der Ordner ändert sich auch im Konto |
| Einen Entwurf speichern | Der Entwurf wird bei Gmail und Outlook in den Entwürfen des Kontos gespeichert; bei einem IMAP-Konto bleibt er auf diesem Computer |

<!-- claim:priv-what-emailops-2 -->
Eine gelöschte Nachricht kommt in den Papierkorb, wo sie sich noch wiederherstellen lässt;
**Endgültig löschen** und **Papierkorb leeren** / **Spam leeren** löschen Nachrichten endgültig, und nur
nachdem Sie es bestätigt haben. Der Lesestatus wird zuerst lokal gesetzt, damit die App offline
funktioniert; das Konto zieht im Hintergrund nach. Alles andere — Labels, Filter, Ordner, die
Sie nicht angefasst haben — bleibt genau so, wie es ist.

<!-- claim:priv-what-emailops-3 -->
Sofern Sie es nicht abschalten, endet jede Nachricht, die Sie aus EmailOps senden, mit einer
kurzen Zeile „Gesendet mit EmailOps“, die auf getemailops.com verlinkt; deaktivieren Sie
**„Gesendet mit EmailOps“ am Ende meiner gesendeten Nachrichten hinzufügen** unter
**Einstellungen → Signaturen**, um ohne sie zu senden. Der Link trägt `utm_source=email_footer`;
das teilt der Analyse der Website nur mit, dass ein Besuch aus einer E-Mail-Fußzeile kam, und
enthält nichts, was Sie oder den Empfänger identifiziert.

## Keine Telemetrie

<!-- claim:priv-no-telemetry-1 -->
Die App sammelt keine Nutzungsstatistiken und sendet keine Absturzberichte. Die einzige
Anfrage, die sie an das Projekt richtet, ist die tägliche Update-Prüfung: Sie lädt von
getemailops.com eine kleine Datei, die die neueste Version nennt. Die Datei und ihre Adresse
sind für alle gleich, und die Anfrage enthält nichts über Sie, Ihre E-Mails oder Ihre Kopie
der App: keine Kennung, keine App-Version, kein Betriebssystem. Der Hoster der Website sieht
die Anfrage wie jeder Webserver, und die Zahl der Anfragen pro Tag ist die einzige Angabe, die
das Projekt darüber hat, wie viele Kopien in Gebrauch sind.
**Nach neuen Versionen suchen** unter **Einstellungen → Datenschutz & Sicherheit** schaltet die
Prüfung ab: Die App kontaktiert getemailops.com dann nie und meldet keine neuen Versionen mehr.
(Der Quellbaum enthält eine optionale OpenTelemetry-Tracing-Funktion für die
lokale Entwicklung; sie wird aus jedem Release-Build herauskompiliert.)

## Standardmäßig lokale KI

<!-- claim:priv-local-ai-1 -->
Das voreingestellte Backend führt Modelle im selben Prozess über eine eingebettete
llama.cpp-Laufzeit aus. Kein Daemon, kein lokaler Server, kein Netzwerk-Socket — das Modell
liest Ihre E-Mails aus dem Prozess, der sie ohnehin schon hat. Klassifizierung, Entwürfe,
Embeddings, Chat sowie Aufgaben- und Gedächtnis-Extraktion laufen alle dort.

<!-- claim:priv-local-ai-2 -->
Der Wechsel zu Ollama hält die Inferenz ebenfalls lokal, nur in einem eigenen Prozess auf
Ihrer Maschine. Nur OpenRouter und ein OpenAI-kompatibler Server auf einem anderen Rechner senden Inhalte vom
Gerät weg. Siehe
[Backend wählen](../ai-features/#choosing-a-backend).

## Schutz vor den E-Mails selbst {#protection-from-the-mail-itself}

<!-- claim:priv-protection-from-1 -->
E-Mail ist eine Angriffsfläche. Die Schutzmechanismen auf Client-Seite:

- **Blockieren entfernter Inhalte** — externe Bilder, Tracking-Pixel und andere entfernte
  Ressourcen werden blockiert, bis Sie sie erlauben. Ein Hinweisbalken je E-Mail lädt sie
  einmalig, oder Sie vertrauen einem bestimmten Absender dauerhaft. Das verhindert, dass
  Absender erfahren, wann und wie oft Sie eine Nachricht geöffnet haben. **Einstellungen →
  Datenschutz & Sicherheit** listet je Konto die Absender auf, denen Sie vertraut haben, mit
  **Widerrufen**, das deren Bilder wieder blockiert. <!-- claim:priv-protection-from-2 -->
- **Junk- und Massenbewertung** — jede Nachricht wird lokal auf Spam und unerwünschte
  Massen-E-Mails bewertet. Ihre Korrekturen („Junk“ / „kein Junk“) trainieren sie. Markierte
  Post wird abgeschwächt oder ausgeblendet, aber nie auf dem Server gelöscht oder verschoben,
  außer Sie bestätigen es ausdrücklich. <!-- claim:priv-protection-from-3 -->
- **Warnungen vor Identitätsmissbrauch** — eine optionale Prüfung, die Nachrichten markiert,
  die scheinbar von jemand anderem stammen. Standardmäßig aus, denn es ist die einzige
  Prüfung, die einem Absender Betrug unterstellt, und sie hat die dünnste Beweislage. <!-- claim:priv-protection-from-4 -->
- **Bereinigtes Rendering** — das HTML der Nachrichten wird vor der Anzeige von Skripten,
  Event-Handlern und eingebetteten Objekten befreit, und zwar auf beiden Seiten der App.
  Anhänge werden nie eigenmächtig geöffnet. <!-- claim:priv-protection-from-5 -->
- **Schutz bei Anhängen** — jeder Anhang, den EmailOps auf der Festplatte speichert, wird als
  von außen empfangen markiert (die Quarantäne unter macOS, das Mark-of-the-Web unter
  Windows), sodass beim ersten Öffnen die Prüfungen des Systems greifen. Das Öffnen eines
  Anhangs, der Code ausführen kann — ein Programm, ein Skript, ein Installationsprogramm, eine
  Verknüpfung oder eine lokale Webseite — verlangt vorher Ihre Bestätigung und nennt die Datei. <!-- claim:priv-protection-from-6 -->

## Die App sperren {#locking-the-app}

<!-- claim:priv-locking-app-1 -->
Legen Sie unter **Einstellungen → Datenschutz & Sicherheit** ein **Hauptpasswort** fest, dann
bleibt EmailOps beim Start gesperrt, bis Sie es eingeben. Es gibt keinen Wiederherstellungsweg
— wenn Sie es vergessen, installieren Sie neu gegen ein frisches Datenverzeichnis und
synchronisieren erneut von Ihrem Anbieter.

<!-- claim:priv-locking-app-2 -->
Zur Klarheit, was das leistet: Es sperrt die Anwendung, es verschlüsselt die Datenbank
**nicht**. Wer Zugriff auf Ihr entsperrtes Benutzerkonto und das Datenverzeichnis hat, kann
die SQLite-Datei direkt lesen. Wenn das zu Ihrem Bedrohungsmodell gehört, nutzen Sie
Festplattenverschlüsselung — FileVault unter macOS, BitLocker unter Windows, LUKS unter
Linux — das ist das richtige Werkzeug dafür.

<!-- claim:priv-locking-app-3 -->
Falsche Versuche werden gebremst: Nach fünf falschen Passwörtern wartet der Sperrbildschirm
30 Sekunden, bevor er ein weiteres annimmt, jedes Mal doppelt so lange bis zu 15 Minuten, und
ein Neustart der App setzt den Zähler nicht zurück.

## All das überprüfen

<!-- claim:priv-auditing-any-1 -->
EmailOps steht unter Apache-2.0 und wird offen entwickelt. Die Aussagen auf dieser Seite sind
anhand des Quellcodes auf
[github.com/emailops/emailops](https://github.com/emailops/emailops) überprüfbar, und das
Netzwerkverhalten ebenso — betreiben Sie die App hinter einem Proxy oder mit `tcpdump` und
vergleichen Sie mit der Tabelle oben. Falls etwas nicht übereinstimmt,
[eröffnen Sie bitte ein Issue](https://github.com/emailops/emailops/issues).
