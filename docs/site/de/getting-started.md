---
title: 'Erste Schritte'
description: 'Der Einrichtungsassistent: KI-Backend wählen, ein Modell herunterladen und das erste Postfach verbinden.'
weight: 20
---

<!-- claim:start-intro-1 -->
Beim ersten Start von EmailOps läuft ein Assistent mit vier Schritten. Er dauert ein paar
Minuten, größtenteils für einen Modell-Download im Hintergrund.

## 1. KI an oder aus

<!-- claim:start-1-ai-1 -->
EmailOps prüft Ihre Hardware und empfiehlt, ob lokale KI aktiviert werden soll. Wählen Sie:

- **KI nutzen** — Chat, Entwürfe, Klassifizierung und semantische Suche laufen alle auf
  dieser Maschine. <!-- claim:start-1-ai-2 -->
- **Einfacher E-Mail-Client** — es wird kein Modell heruntergeladen und nie ein KI-Aufruf
  gemacht. Sie können die KI später unter **Einstellungen → KI: Backend & Modelle** einschalten
  und ebenso leicht wieder aus. <!-- claim:start-1-ai-3 -->

## 2. KI-Backend und Modell

<!-- claim:start-2-ai-1 -->
Wenn Sie die KI aktiviert haben, wählen Sie, wo die Inferenz stattfindet:

| Backend | Was es bedeutet |
|---|---|
| **In der App (lokal)** | Der Standard. Eine in EmailOps eingebettete llama.cpp-Laufzeit. Kein Daemon, keine Einrichtung, kein Netzwerk. |
| **Ollama (lokal)** | Nutzt Ihren vorhandenen Ollama-Server unter `http://localhost:11434`. |
| **OpenRouter (entfernt)** | Sendet Prompts an eine kostenpflichtige Cloud-API. Optional, pro Funktion, standardmäßig aus. |

<!-- claim:start-2-ai-2 -->
Wählen Sie beim eingebauten Backend ein Chat-Modell aus dem Katalog. EmailOps wählt das größte
Modell vor, das Ihre Maschine bequem ausführen kann — die Empfehlung hängt also vom gefundenen
Arbeitsspeicher ab: auf einer 16-GB-Maschine ist das **Qwen 3.5 4B**, rund 3 GB Download und
etwa 8 GB Arbeitsspeicher zum Ausführen; eine größere Maschine bekommt ein größeres Modell aus
demselben Katalog angeboten. Alle empfohlenen Modelle unterstützen die Tool-Aufrufe, auf die
der Chat angewiesen ist. Modelle, die für Ihren Systemspeicher zu groß sind, werden ausgegraut.
Der Download läuft im Hintergrund — Sie können im Assistenten weitermachen.

<!-- claim:start-2-ai-3 -->
Welcher Speicher zählt, hängt von der Maschine ab: **Unified Memory** auf einem Apple-Silicon-
Mac, der **VRAM Ihrer GPU** auf einem Windows- oder Linux-Rechner mit dedizierter Karte, und
der System-RAM, wenn keine GPU vorhanden ist. Der
[Modellkatalog](../ai-features/#the-model-catalog) nennt den Wert für jedes Modell.

<!-- claim:start-2-ai-4 -->
Das Embedding-Modell hinter der semantischen Suche (**Nomic Embed Text v1.5**, ~80 MB) ist
unter macOS in der App enthalten — für die Suche gibt es also nichts herunterzuladen.

## 3. Layout des Posteingangs

<!-- claim:start-3-inbox-1 -->
Wählen Sie die Aufteilung — **geteilt** (Liste links, Nachricht rechts) oder **volle Breite**
(ein Bereich nach dem anderen). Jederzeit änderbar unter **Einstellungen → Erscheinungsbild**,
zusammen mit der Sprache der Oberfläche (Deutsch, Englisch, Spanisch, Französisch).

## 4. Ein Konto verbinden

<!-- claim:start-4-connect-1 -->
Der letzte Schritt fügt Ihr erstes Postfach hinzu. EmailOps unterstützt:

- **Gmail** — melden Sie sich im Browser an und erteilen Sie den Zugriff. Die Tokens gehen
  direkt in den Schlüsselbund des Systems. <!-- claim:start-4-connect-2 -->
- **Outlook / Microsoft 365** — derselbe Browser-Ablauf, über die Microsoft-Graph-API. <!-- claim:start-4-connect-3 -->
- **IMAP / SMTP** — iCloud, Yahoo, Fastmail, ProtonMail Bridge oder ein beliebiger eigener
  Server. Serverdaten und Zugangsdaten direkt eingeben. <!-- claim:start-4-connect-4 -->

<!-- claim:start-4-connect-5 -->
Weitere Konten fügen Sie jederzeit über **Konto hinzufügen** in der Seitenleiste hinzu. Mit mehreren
verbundenen Konten erhalten Sie zusätzlich zu den Einzelansichten einen vereinten Posteingang
„Alle Konten“.

## Nach dem Assistenten

### Die erste Synchronisierung dauert

<!-- claim:start-after-wizard-first-sync-1 -->
EmailOps lädt Ihre E-Mails in eine lokale Datenbank, und der erste Durchlauf muss alles von
Grund auf holen. Wie lange das dauert, hängt von der Größe des Postfachs ab — ein paar Minuten
bei einem kleinen Konto, deutlich länger bei einem mit jahrelanger Historie und großen
Anhängen. Es läuft im Hintergrund, und die ersten Nachrichten erscheinen innerhalb von Sekunden — die
E-Mails werden abschnittsweise geladen, während das Postfach durchgegangen wird, und nicht
erst danach — sodass Sie bereits Eingetroffenes lesen und durchsuchen können, während der
Rest nachzieht.

<!-- claim:start-after-wizard-first-sync-2 -->
Das sind einmalige Kosten. Jede spätere Synchronisierung ist **inkrementell**: Sie fragt beim
Anbieter nur ab, was sich seither geändert hat, ist daher in Sekunden fertig und läuft
unauffällig nach Zeitplan. Bei aktivierter KI arbeiten auch Klassifizierung und Embeddings
beim ersten Lauf den Rückstand ab und fassen danach nur noch neue E-Mails an.

<!-- claim:start-after-wizard-first-sync-3 -->
Sobald die erste Synchronisierung abgeschlossen ist:

1. Die **Klassifizierung** beginnt, neue E-Mails nach Priorität, Absicht und Thema zu
   kennzeichnen — siehe [KI-Funktionen](../ai-features/#classification). <!-- claim:start-after-wizard-first-sync-4 -->
2. **Embeddings** werden im Hintergrund erzeugt, damit die semantische Suche eine Grundlage
   hat. Fortschritt und Neuaufbau des Index finden Sie unter
   **Einstellungen → KI-Suche**. <!-- claim:start-after-wizard-first-sync-5 -->
3. Erwägen Sie ein **Hauptpasswort** unter **Einstellungen → Datenschutz & Sicherheit**, wenn
   die App beim Start gesperrt sein soll — siehe
   [Datenschutz und Sicherheit](../privacy-security/). <!-- claim:start-after-wizard-first-sync-6 -->

<!-- claim:start-after-wizard-first-sync-7 -->
Klassifizierung und Embeddings berücksichtigen beide **KI-Verarbeitung begrenzen**
(**Einstellungen → KI: Backend & Modelle**), sodass ein zehn Jahre altes Archiv nur auf
ausdrücklichen Wunsch verarbeitet wird.
