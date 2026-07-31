---
title: 'Erste Schritte'
description: 'Der Einrichtungsassistent: KI-Backend wählen, ein Modell herunterladen und das erste Postfach verbinden.'
weight: 20
---

Beim ersten Start von EmailOps läuft ein Assistent mit vier Schritten. Er dauert ein paar
Minuten, größtenteils für einen Modell-Download im Hintergrund.

## 1. KI an oder aus

EmailOps prüft Ihre Hardware und empfiehlt, ob lokale KI aktiviert werden soll. Wählen Sie:

- **KI aktiviert** — Chat, Entwürfe, Klassifizierung und semantische Suche laufen alle auf
  dieser Maschine.
- **Einfacher E-Mail-Client** — es wird kein Modell heruntergeladen und nie ein KI-Aufruf
  gemacht. Sie können die KI später unter **Einstellungen → KI: Backend & Modelle** einschalten
  und ebenso leicht wieder aus.

## 2. KI-Backend und Modell

Wenn Sie die KI aktiviert haben, wählen Sie, wo die Inferenz stattfindet:

| Backend | Was es bedeutet |
|---|---|
| **In der App (lokal)** | Der Standard. Eine in EmailOps eingebettete llama.cpp-Laufzeit. Kein Daemon, keine Einrichtung, kein Netzwerk. |
| **Ollama (lokal)** | Nutzt Ihren vorhandenen Ollama-Server unter `http://localhost:11434`. |
| **OpenRouter (entfernt)** | Sendet Prompts an eine kostenpflichtige Cloud-API. Optional, pro Funktion, standardmäßig aus. |

Wählen Sie beim eingebauten Backend ein Chat-Modell aus dem Katalog. **Qwen 3.5 4B** ist der
empfohlene Standard: rund 3 GB Download, benötigt etwa 8 GB Arbeitsspeicher zum Ausführen und
unterstützt die Tool-Aufrufe, auf die der Chat angewiesen ist. Modelle, die für Ihren
Systemspeicher zu groß sind, werden ausgegraut. Der Download läuft im Hintergrund — Sie können
im Assistenten weitermachen.

Welcher Speicher zählt, hängt von der Maschine ab: **Unified Memory** auf einem Apple-Silicon-
Mac, der **VRAM Ihrer GPU** auf einem Windows- oder Linux-Rechner mit dedizierter Karte, und
der System-RAM, wenn keine GPU vorhanden ist. Der
[Modellkatalog](../ai-features/#the-model-catalog) nennt den Wert für jedes Modell.

Das Embedding-Modell hinter der semantischen Suche (**Nomic Embed Text v1.5**, ~80 MB) ist
unter macOS in der App enthalten — für die Suche gibt es also nichts herunterzuladen.

## 3. Layout des Posteingangs

Wählen Sie die Aufteilung — **geteilt** (Liste links, Nachricht rechts) oder **volle Breite**
(ein Bereich nach dem anderen). Jederzeit änderbar unter **Einstellungen → Erscheinungsbild**,
zusammen mit der Sprache der Oberfläche (Deutsch, Englisch, Spanisch, Französisch).

## 4. Ein Konto verbinden

Der letzte Schritt fügt Ihr erstes Postfach hinzu. EmailOps unterstützt:

- **Gmail** — melden Sie sich im Browser an und erteilen Sie den Zugriff. Die Tokens gehen
  direkt in den Schlüsselbund des Systems.
- **Outlook / Microsoft 365** — derselbe Browser-Ablauf, über die Microsoft-Graph-API.
- **IMAP / SMTP** — iCloud, Yahoo, Fastmail, ProtonMail Bridge oder ein beliebiger eigener
  Server. Serverdaten und Zugangsdaten direkt eingeben.

Weitere Konten fügen Sie jederzeit über **Konto hinzufügen** in der Seitenleiste hinzu. Mit mehreren
verbundenen Konten erhalten Sie zusätzlich zu den Einzelansichten einen vereinten Posteingang
„Alle Konten“.

## Nach dem Assistenten

### Die erste Synchronisierung dauert

EmailOps lädt Ihre E-Mails in eine lokale Datenbank, und der erste Durchlauf muss alles von
Grund auf holen. Wie lange das dauert, hängt von der Größe des Postfachs ab — ein paar Minuten
bei einem kleinen Konto, deutlich länger bei einem mit jahrelanger Historie und großen
Anhängen. Es läuft im Hintergrund, und Sie können bereits Eingetroffenes lesen und
durchsuchen, während der Rest nachzieht.

Das sind einmalige Kosten. Jede spätere Synchronisierung ist **inkrementell**: Sie fragt beim
Anbieter nur ab, was sich seither geändert hat, ist daher in Sekunden fertig und läuft
unauffällig nach Zeitplan. Bei aktivierter KI arbeiten auch Klassifizierung und Embeddings
beim ersten Lauf den Rückstand ab und fassen danach nur noch neue E-Mails an.

Sobald die erste Synchronisierung abgeschlossen ist:

1. Die **Klassifizierung** beginnt, neue E-Mails nach Priorität, Absicht und Thema zu
   kennzeichnen — siehe [KI-Funktionen](../ai-features/#classification).
2. **Embeddings** werden im Hintergrund erzeugt, damit die semantische Suche eine Grundlage
   hat. Fortschritt und Neuaufbau des Index finden Sie unter
   **Einstellungen → KI-Suche**.
3. Erwägen Sie ein **Hauptpasswort** unter **Einstellungen → Datenschutz & Sicherheit**, wenn
   die App beim Start gesperrt sein soll — siehe
   [Datenschutz und Sicherheit](../privacy-security/).

Klassifizierung und Embeddings berücksichtigen beide **KI-Verarbeitung auf neuere
E-Mails beschränken**
(**Einstellungen → KI: Backend & Modelle**), sodass ein zehn Jahre altes Archiv nur auf
ausdrücklichen Wunsch verarbeitet wird.
