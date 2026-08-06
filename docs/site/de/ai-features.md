---
title: 'KI-Funktionen'
description: 'Mit dem Postfach chatten, Antworten erzeugen, E-Mails klassifizieren, Aufgaben extrahieren — alles auf einem Modell, das Sie kontrollieren.'
weight: 40
---

Alle KI-Funktionen unten laufen über das von Ihnen gewählte Backend, und jede lässt sich
einzeln abschalten. Mit dem voreingestellten integrierten Backend verlässt kein Prompt und
keine E-Mail jemals Ihre Maschine.

## Backend wählen {#choosing-a-backend}

**Einstellungen → KI: Backend & Modelle** legt fest, wo die Inferenz stattfindet:

- **In der App (lokal)** — eine eingebettete llama.cpp-Laufzeit. Nichts zu installieren, kein
  Daemon, kein Netzwerkverkehr. Das ist der Standard. Sie nutzt automatisch Ihre GPU, wenn
  eine vorhanden ist — Metal auf Apple Silicon, Vulkan unter Windows und Linux — und sonst die
  CPU. Auf dem Mac wird Apple Silicon (M1 oder neuer) vorausgesetzt; auf einem Intel-Mac bleibt
  sie nicht verfügbar.
- **Ollama (lokal)** — ein Ollama-Server, den Sie bereits unter `http://localhost:11434`
  betreiben. Praktisch, wenn Sie eine gemeinsame Modellbibliothek pflegen. Beachten Sie: Auf
  einem Intel-Mac erhält auch Ollama keine GPU-Beschleunigung und ist entsprechend langsam.
- **OpenRouter (entfernt)** — eine kostenpflichtige Cloud-API. Erfordert einen API-Schlüssel,
  unterstützt ein monatliches Budgetlimit und sendet E-Mail-Inhalte an einen Dritten — daher
  bleibt sie aus, bis Sie sie aktivieren.

### Der Modellkatalog {#the-model-catalog}

Das integrierte Backend lädt Modelle aus einem kuratierten Katalog, jedes auf eine geprüfte
Prüfsumme festgelegt:

| Modell | Downloadgröße | Benötigter Speicher zur Ausführung |
|---|---|---|
| Qwen 3.5 4B *(empfohlen)* | ~3,0 GB | 8 GB |
| Qwen 3.5 4B Q8 | ~4,6 GB | 12 GB |
| Qwen 3.5 9B | ~5,7 GB | 16 GB |
| Gemma 4 12B Instruct | ~6,7 GB | 16 GB |
| Qwen 3.5 27B | ~17,6 GB | 24 GB |
| Qwen 3.6 35B A3B | ~22,4 GB | 32 GB |
| Nomic Embed Text v1.5 *(Embeddings, mitgeliefert)* | ~84 MB | 1 GB |

Die rechte Spalte ist der Spitzenspeicher während der Antwort — Gewichte plus Kontextfenster —
und damit stets mehr als der Download. **In welchen** Speicher es passen muss, hängt von Ihrer
Hardware ab:

- **Apple Silicon** — Unified Memory, geteilt zwischen CPU und GPU, angesprochen über Metal.
  Vergleichen Sie den Wert mit dem Gesamtspeicher Ihres Macs.
- **Eine GPU unter Windows oder Linux** — der **VRAM** der Karte, nicht Ihr System-RAM,
  angesprochen über Vulkan. Eine 8-GB-Karte fährt die 8-GB-Zeile und nichts darüber, egal wie
  viel RAM die Maschine hat.
- **Keine GPU** — System-RAM, auf der CPU. Es funktioniert; es ist nur langsamer.

Modelle, die für Ihren Systemspeicher zu groß sind, erscheinen in der Auswahl ausgegraut.
Größere Modelle antworten besser und laufen langsamer — beginnen Sie mit dem empfohlenen und
steigen Sie nur auf, wenn die Hardware Luft hat. Die vollständigen Anforderungen stehen unter
[Installation](../installation/#with-local-ai).

### Leistungsstellschrauben

- **Modell geladen halten** — wie lange das Modell zwischen zwei Anfragen im Speicher bleibt
  (Standard 30 Minuten). Höhere Werte ersparen das langsame Nachladen; `0` entlädt es sofort
  und gibt den Speicher für andere Apps frei.
- **Kontextfenster** — wie viele Token das Modell pro Anfrage berücksichtigen kann. Größer
  fasst mehr abgerufene E-Mails und kostet mehr Speicher — das ist die erste Stellschraube
  zum Verkleinern, wenn ein Modell nur knapp passt.
- **Denkmodus** — Chain-of-Thought bei unterstützten Modellen. Langsamer, genauer, und Sie können
  die Argumentationsspur ein- oder ausblenden.
- **KI-Verarbeitung auf neuere E-Mails beschränken** — überspringt Embeddings und
  Klassifizierung für E-Mails, die älter als N Tage sind.

## Mit dem Postfach chatten

Fragen Sie in natürlicher Sprache — *„Was hat der Anwalt zum Vertrag gesagt?“*, *„Fasse diesen
Thread zusammen“*, *„Wer schuldet mir noch eine Antwort?“* — und erhalten Sie eine Antwort mit
Angabe der Quell-E-Mails. Die Antworten erscheinen im Stream, während sie erzeugt werden.

Unter der Haube kombiniert der Chat Retrieval (semantische Suche über Ihre indexierten
E-Mails) mit Tool-Aufrufen (direkte Abfragen der Datenbank). Der Routing-Modus ist
einstellbar:

- **Immer RAG zuerst** — der Standard; Kontext abrufen, dann antworten.
- **Auto** — eine Heuristik wählt je Frage zwischen Retrieval und Tools.
- **Immer Tools zuerst** — direkt zu den strukturierten Abfragen.

Fortgeschrittene können den System-Prompt und die Retrieval-Prompts (Query-Umschreibung,
Reranking) unter **Einstellungen → KI: Backend & Modelle → Chat-Prompts** bearbeiten.

## KI-Entwürfe

Ein Button **KI-Entwurf** neben „Allen antworten“ schreibt eine Antwort, die im gerade
geöffneten Thread verankert ist. Konfigurieren Sie eine **Persona** (ein Satz dazu, als wer
die KI schreibt), einen **Schreibstil** sowie Standardton und -länge — oder ersetzen Sie die
gesamte Prompt-Vorlage. Entwürfe landen im Editor, damit Sie sie vor dem Senden prüfen.

## Klassifizierung {#classification}

Jede eingehende E-Mail wird entlang dreier Achsen gekennzeichnet — **Priorität**, **Absicht**
und **Thema** — sodass sich der Posteingang praktisch selbst sortiert und die intelligenten
Filter etwas zum Filtern haben.

Die Klassifizierung arbeitet in zwei Schichten:

1. **Regeln** greifen bei Absender- oder Betreffmustern (`*@*.beehiiv.com`, `*Rechnung*`) und
   vergeben Kennzeichnungen sofort, ohne Modellaufruf.
2. **Das Modell** übernimmt alles, was die Regeln nicht abdecken, mit einem
   Anweisungs-Prompt, den Sie bearbeiten können.

Sie bestimmen, welche Gmail-Kategorien klassifiziert werden, können nach einer
Prompt-Änderung alles neu klassifizieren und nicht klassifizierte E-Mails bei Bedarf
nachholen.

## Semantische Suche

E-Mails werden lokal eingebettet, damit die Suche nach Bedeutung statt nur nach Stichwörtern
trifft — beschreiben Sie, woran Sie sich erinnern, und EmailOps findet es. Das treibt auch
„Ähnliche finden“ und den Retrieval-Schritt im Chat an. Wählen Sie unter
**Einstellungen → KI-Suche**, welche Kategorien eingebettet werden, und bauen Sie den Index
nach einem Wechsel des Embedding-Modells von Grund auf neu.

## Übersetzung

Bei E-Mails in einer anderen Sprache und im Verfassen-Fenster erscheinen
Übersetzen-Schaltflächen. Der Übersetzungs-Prompt ist wie die anderen bearbeitbar.

## Aufgaben

*Experimentell.* EmailOps durchsucht E-Mails nach Handlungspunkten, Zusagen und Fristen und
sammelt sie in einem Aufgaben-Bereich. Da echte Zusagen meist in dem stehen, was **Sie**
geschrieben haben, gibt es einen Modus „nur aus selbst geschriebenen E-Mails lernen“. Sie
können Absender und Kennzeichnungen ausschließen (Newsletter sind standardmäßig
ausgeschlossen), Aufgaben pro E-Mail begrenzen, den Rückblickzeitraum einschränken und ältere
E-Mails bei Bedarf nacharbeiten lassen.

## Gedächtnis

*Experimentell.* Fakten, die der Assistent über Ihre Kontakte, Domains und Projekte lernt,
werden als Langzeitkontext gespeichert, damit der Chat nicht jedes Mal bei null beginnt.
Kandidaten-Fakten werden bewertet und ab einem Schwellenwert übernommen; schlecht bewertete
laufen aus. Alles Gelernte ist einsehbar, und das gesamte Teilsystem hat einen Hauptschalter.

## Linsen

*Experimentell.* Typisierte Sichten auf Ihr Postfach — gespeicherte, per KI extrahierte
strukturierte Projektionen (etwa „alle Rechnungen mit Betrag und Fälligkeit“), die Sie in der
Seitenleiste anlegen und ausführen.

## Alles abschalten

**Einstellungen → KI: Backend & Modelle → KI-Funktionen** ist ein Hauptschalter. Schalten Sie
ihn aus, und EmailOps läuft als reiner E-Mail-Client: kein Chat, keine Klassifizierung, keine
Embeddings, kein geladenes Modell. Ihre vorhandenen lokalen KI-Daten bleiben erhalten, falls
Sie ihn wieder einschalten.
