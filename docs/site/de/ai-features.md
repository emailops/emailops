---
title: 'KI-Funktionen'
description: 'Mit dem Postfach chatten, Antworten erzeugen, E-Mails klassifizieren, Aufgaben extrahieren — alles auf einem Modell, das Sie kontrollieren.'
weight: 40
---

<!-- claim:ai-intro-1 -->
Alle KI-Funktionen unten laufen über das von Ihnen gewählte Backend, und jede lässt sich
einzeln abschalten. Mit dem voreingestellten integrierten Backend verlässt kein Prompt und
keine E-Mail jemals Ihre Maschine.

## Backend wählen {#choosing-a-backend}

<!-- claim:ai-choosing-backend-1 -->
**Einstellungen → KI: Backend & Modelle** legt fest, wo die Inferenz stattfindet:

- **In der App** — eine eingebettete llama.cpp-Laufzeit. Nichts zu installieren, kein
  Daemon, kein Netzwerkverkehr. Das ist der Standard. Sie nutzt automatisch Ihre GPU, wenn
  eine vorhanden ist — Metal auf Apple Silicon, Vulkan unter Windows und Linux — und sonst die
  CPU. Auf dem Mac wird Apple Silicon (M1 oder neuer) vorausgesetzt; auf einem Intel-Mac bleibt
  sie nicht verfügbar. <!-- claim:ai-choosing-backend-2 -->
- **Ollama** — ein Ollama-Server, den Sie bereits unter `http://localhost:11434`
  betreiben. Praktisch, wenn Sie eine gemeinsame Modellbibliothek pflegen. Beachten Sie: Auf
  einem Intel-Mac erhält auch Ollama keine GPU-Beschleunigung und ist entsprechend langsam. <!-- claim:ai-choosing-backend-3 -->
- **OpenRouter** — eine kostenpflichtige Cloud-API. Erfordert einen API-Schlüssel,
  unterstützt ein monatliches Budgetlimit und sendet E-Mail-Inhalte an einen Dritten — daher
  bleibt sie aus, bis Sie sie aktivieren. <!-- claim:ai-choosing-backend-4 -->

### Der Modellkatalog {#the-model-catalog}

<!-- claim:ai-choosing-backend-model-catalog-1 -->
Das integrierte Backend lädt Modelle aus einem kuratierten Katalog, jedes auf eine geprüfte
Prüfsumme festgelegt:

<!-- generated:model-catalog -->
| Modell | Downloadgröße | Von EmailOps verlangter Speicher |
|---|---|---|
| Qwen 3.5 4B | ~3,0 GB | 8 GB |
| Qwen 3.5 4B Q8 | ~4,6 GB | 12 GB |
| Qwen 3.5 9B | ~5,7 GB | 16 GB |
| Gemma 4 12B Instruct | ~6,7 GB | 16 GB |
| Qwen 3.5 27B | ~17,6 GB | 24 GB |
| Qwen 3.6 35B A3B | ~22,4 GB | 32 GB |
| Nomic Embed Text v1.5 *(Embeddings, mitgeliefert)* | ~84 MB | 1 GB |
<!-- /generated:model-catalog -->

<!-- claim:ai-choosing-backend-model-catalog-2 -->
Die rechte Spalte ist der Speicher, den EmailOps verlangt, bevor es ein Modell anbietet — ein
bewusst großzügiger Puffer, nicht der tatsächliche Verbrauch. Gemessene Spitze während einer
Antwort, mit dem Kontext eines 16-GB-Macs: rund 3,7 GB für Qwen 3.5 4B, 4,3 GB für die
8-Bit-Variante, 5,6 GB für Qwen 3.5 9B und 7,1 GB für Gemma 4 12B. **In welchen** Speicher es
passen muss, hängt von Ihrer Hardware ab:

- **Apple Silicon** — Unified Memory, geteilt zwischen CPU und GPU, angesprochen über Metal.
  Vergleichen Sie den Wert mit dem Gesamtspeicher Ihres Macs. <!-- claim:ai-choosing-backend-model-catalog-3 -->
- **Eine GPU unter Windows oder Linux** — der **VRAM** der Karte, nicht Ihr System-RAM,
  angesprochen über Vulkan. Eine 8-GB-Karte fährt die 8-GB-Zeile und nichts darüber, egal wie
  viel RAM die Maschine hat. <!-- claim:ai-choosing-backend-model-catalog-4 -->
- **Keine GPU** — System-RAM, auf der CPU. Es funktioniert; es ist nur langsamer. <!-- claim:ai-choosing-backend-model-catalog-5 -->

<!-- claim:ai-choosing-backend-model-catalog-6 -->
Modelle, die für Ihren Systemspeicher zu groß sind, erscheinen in der Auswahl ausgegraut.
Ein Modell trägt die Markierung **Empfohlen**, ausgewählt für die Maschine, an der Sie sitzen:
EmailOps betrachtet den Systemspeicher und, sofern eine dedizierte Grafikkarte vorhanden ist,
auch deren Speicher, und schlägt dann das größte Modell vor, das bequem hineinpasst. Ein
Laptop und eine Workstation sehen deshalb unterschiedliche Vorschläge. Größere Modelle
antworten besser und laufen langsamer — die Markierung ist ein Ausgangspunkt, keine Regel.
Die vollständigen Anforderungen stehen unter [Installation](../installation/#with-local-ai).

### Leistungsstellschrauben

- **Modell geladen halten** — wie lange das Modell zwischen zwei Anfragen im Speicher bleibt
  (Standard 30 Minuten). Höhere Werte ersparen das langsame Nachladen; `0` entlädt es sofort
  und gibt den Speicher für andere Apps frei. <!-- claim:ai-choosing-backend-performance-knobs-1 -->
- **Kontextfenster** — wie viele Token das Modell pro Anfrage berücksichtigen kann. Größer
  fasst mehr abgerufene E-Mails und kostet mehr Speicher — das ist die erste Stellschraube
  zum Verkleinern, wenn ein Modell nur knapp passt. <!-- claim:ai-choosing-backend-performance-knobs-2 -->
- **Denkmodus** — Chain-of-Thought bei unterstützten Modellen. Langsamer, genauer, und Sie können
  die Argumentationsspur ein- oder ausblenden. <!-- claim:ai-choosing-backend-performance-knobs-3 -->
- **KI-Verarbeitung begrenzen** — begrenzt, was Embeddings und Klassifizierung abdecken: alle
  E-Mails eines Kontos bis zu einem E-Mail-Limit (standardmäßig 1000) und bei größeren Konten
  nur E-Mails, die jünger als ein Tageslimit sind (standardmäßig 365 Tage). <!-- claim:ai-choosing-backend-performance-knobs-4 -->

## Mit dem Postfach chatten

<!-- claim:ai-chat-mailbox-1 -->
Fragen Sie in natürlicher Sprache — *„Was hat der Anwalt zum Vertrag gesagt?“*, *„Fasse diesen
Thread zusammen“*, *„Wer schuldet mir noch eine Antwort?“* — und erhalten Sie eine Antwort mit
Angabe der Quell-E-Mails. Die Antworten erscheinen im Stream, während sie erzeugt werden.

<!-- claim:ai-chat-mailbox-2 -->
Der Chat sitzt in einem größenveränderlichen Panel rechts neben dem Posteingang, sodass Sie
beim Fragen weiterlesen können; für längere Sitzungen gibt es zusätzlich eine
Vollbildansicht. Ist eine E-Mail geöffnet, bietet das Panel diesen Thread über einen
entfernbaren Chip als Kontext an: Fragen zu dieser E-Mail beantwortet es aus dem Thread,
eine Frage zum übrigen Postfach (*„Was ist heute gekommen?“*) durchsucht weiterhin das
Postfach. Dieser Kontext gilt für genau eine Frage und wird nie in der Unterhaltung
gespeichert — Sie können sich also innerhalb eines Chats zwischen E-Mails bewegen.

<!-- claim:ai-chat-mailbox-3 -->
Der Chat durchsucht immer ein Konto, und eine Auswahl benennt welches — so stammt eine
Antwort nie unbemerkt aus dem falschen Postfach. Jedes Konto behält seine eigene
Unterhaltung, solange die App geöffnet ist; ein Kontowechsel bringt Sie also dorthin zurück,
wo Sie aufgehört haben, und nicht zu einem leeren Chat.

<!-- claim:ai-chat-mailbox-4 -->
Unter der Haube kombiniert der Chat Retrieval (semantische Suche über Ihre indexierten
E-Mails) mit Tool-Aufrufen (direkte Abfragen der Datenbank). Der Routing-Modus ist
einstellbar:

- **Immer RAG zuerst** — der Standard; Kontext abrufen, dann antworten. <!-- claim:ai-chat-mailbox-5 -->
- **Auto** — eine Heuristik entscheidet je Frage, ob zuerst Kontext abgerufen wird. <!-- claim:ai-chat-mailbox-6 -->
- **Immer Tools zuerst** — ohne Retrieval direkt zu den strukturierten Abfragen. <!-- claim:ai-chat-mailbox-7 -->

<!-- claim:ai-chat-mailbox-8 -->
In jedem Modus bleiben die Tools verfügbar; der Modus entscheidet nur, ob vor der Antwort
Kontext abgerufen wird.

<!-- claim:ai-chat-mailbox-9 -->
Fortgeschrittene können den System-Prompt und die Retrieval-Prompts (Query-Umschreibung,
Reranking) unter **Einstellungen → KI: Backend & Modelle → Chat-Prompts** bearbeiten.

## KI-Entwürfe

<!-- claim:ai-ai-drafts-1 -->
Ein Button **KI-Entwurf** neben „Allen antworten“ schreibt eine Antwort, die im gerade
geöffneten Thread verankert ist. Konfigurieren Sie eine **Persona** (ein Satz dazu, als wer
die KI schreibt) und einen **Schreibstil** — oder ersetzen Sie die gesamte Prompt-Vorlage. Entwürfe landen im Editor, damit Sie sie vor dem Senden prüfen.

## Klassifizierung {#classification}

<!-- claim:ai-classification-1 -->
Jede eingehende E-Mail wird entlang dreier Achsen gekennzeichnet — **Priorität**, **Absicht**
und **Thema** — sodass sich der Posteingang praktisch selbst sortiert und die intelligenten
Filter etwas zum Filtern haben.

<!-- claim:ai-classification-2 -->
Die Klassifizierung arbeitet in zwei Schichten:

1. **Regeln** greifen bei Absender- oder Betreffmustern (`*@*.beehiiv.com`, `*Rechnung*`) und
   vergeben Kennzeichnungen sofort, ohne Modellaufruf. <!-- claim:ai-classification-3 -->
2. **Das Modell** übernimmt alles, was die Regeln nicht abdecken, mit einem
   Anweisungs-Prompt, den Sie bearbeiten können. <!-- claim:ai-classification-4 -->

<!-- claim:ai-classification-5 -->
Sie bestimmen, welche Gmail-Kategorien klassifiziert werden, können nach einer
Prompt-Änderung alles neu klassifizieren und nicht klassifizierte E-Mails bei Bedarf
nachholen.

## Tag-Board {#tag-board}

<!-- claim:tag-board-dimensions -->
Das **Tag-Board** (unter **Ansichten** in der Seitenleiste, neben dem Posteingang) macht aus
diesen Kennzeichnungen ein Board. Wählen Sie eine Dimension — **Unternehmen**, **Priorität**,
**Absicht** oder **Thema** — und jeder Kennzeichnungswert wird zu einem Block mit seinen
Threads; unter **Alle Konten** gibt es je Konto und Kennzeichnung einen Block. Ein Thread
steht in genau einem Block, unter der Kennzeichnung seiner zuletzt klassifizierten Nachricht.

<!-- claim:ai-tag-board-2 -->
Die Blöcke sind danach geordnet, wie viel Aufmerksamkeit eine Kennzeichnung tatsächlich
bekommt — wie oft Sie ihre Threads beantworten und lesen, mit mehr Gewicht auf jüngerer
Aktivität — Werbung und Benachrichtigungen stehen zuletzt. Die intelligenten Filter in der
Seitenleiste folgen derselben Reihenfolge. Ziehen Sie Blöcke, um sie umzuordnen (die
Reihenfolge wird je Dimension gemerkt), blenden Sie ein Tag über sein ⋮-Menü aus — das
nächste Tag rückt an seine Stelle, und der Filter verschwindet auch aus der Seitenleiste —
und holen Sie ausgeblendete Tags über den Link **Ausgeblendete Tags anzeigen** zurück.

<!-- claim:tag-board-toolbar -->
Die Werkzeugleiste grenzt das Board nach Zeitraum ein (**Heute**, **Gestern**, **Letzte 7
Tage** oder ein eigener Datumsbereich), nach Gmail-Kategorie, nach Tag-Name und mit
demselben Schalter **Spam-Nachrichten ausblenden** wie im Posteingang; zwei Symbole legen die
Blockbreite fest. Ein Klick auf eine Karte öffnet den Thread im Lesebereich, ihr ⋮-Menü
bietet dieselben Aktionen wie eine Zeile im Posteingang, und das Chat-Symbol im Lesebereich
startet eine Unterhaltung mit diesem Thread als Kontext.

<!-- claim:ai-tag-board-4 -->
Das Board braucht die Klassifizierung: Es bleibt leer, bis E-Mails gekennzeichnet sind, und
wird bei ausgeschalteten KI-Funktionen nicht angezeigt.

## Semantische Suche

<!-- claim:ai-semantic-search-1 -->
E-Mails werden lokal eingebettet, damit die Suche nach Bedeutung statt nur nach Stichwörtern
trifft — beschreiben Sie, woran Sie sich erinnern, und EmailOps findet es. Das treibt auch den Retrieval-Schritt im Chat an. Wählen Sie unter
**Einstellungen → KI-Suche**, welche Kategorien eingebettet werden, und bauen Sie den Index
nach einem Wechsel des Embedding-Modells von Grund auf neu.

## Übersetzung

<!-- claim:ai-translation-1 -->
Bei E-Mails in einer anderen Sprache und im Verfassen-Fenster erscheinen
Übersetzen-Schaltflächen. Der Übersetzungs-Prompt ist wie die anderen bearbeitbar.

## Aufgaben

<!-- claim:ai-tasks-1 -->
*Experimentell.* EmailOps durchsucht E-Mails nach Handlungspunkten, Zusagen und Fristen und
sammelt sie in einem Aufgaben-Bereich. Da echte Zusagen meist in dem stehen, was **Sie**
geschrieben haben, gibt es einen Modus „nur aus selbst geschriebenen E-Mails lernen“. Sie
können Absender und Kennzeichnungen ausschließen (Newsletter sind standardmäßig
ausgeschlossen), Aufgaben pro E-Mail begrenzen, den Rückblickzeitraum einschränken und ältere
E-Mails bei Bedarf nacharbeiten lassen.

## Gedächtnis

<!-- claim:ai-memory-1 -->
*Experimentell.* Fakten, die der Assistent über Ihre Kontakte, Domains und Projekte lernt,
werden als Langzeitkontext gespeichert, damit der Chat nicht jedes Mal bei null beginnt.
Kandidaten-Fakten werden bewertet und ab einem Schwellenwert übernommen; schlecht bewertete
laufen aus. Alles Gelernte ist einsehbar, und das gesamte Teilsystem hat einen Hauptschalter.

## Linsen

<!-- claim:ai-lenses-1 -->
*Experimentell.* Typisierte Sichten auf Ihr Postfach — gespeicherte, per KI extrahierte
strukturierte Projektionen (etwa „alle Rechnungen mit Betrag und Fälligkeit“), die Sie in der
Seitenleiste anlegen und ausführen.

## Alles abschalten

<!-- claim:ai-turning-off-1 -->
**Einstellungen → KI: Backend & Modelle → KI-Funktionen** ist ein Hauptschalter. Schalten Sie
ihn aus, und EmailOps läuft als reiner E-Mail-Client: kein Chat, keine Klassifizierung, keine
Embeddings, kein geladenes Modell. Ihre vorhandenen lokalen KI-Daten bleiben erhalten, falls
Sie ihn wieder einschalten.
