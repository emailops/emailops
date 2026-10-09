---
title: 'KI-Funktionen'
description: 'Mit dem Postfach chatten, Antworten erzeugen, E-Mails klassifizieren, Aufgaben extrahieren — alles auf einem Modell, das Sie kontrollieren.'
weight: 40
nav:
  choosing-a-backend: settings/ai
  the-model-catalog: settings/ai
  performance-knobs: settings/ai
  chat-with-your-mailbox: view/chat
  ai-drafts: settings/aidrafts
  classification: settings/classification
  tag-board: view/tagboard
  semantic-search: settings/aisearch
  translation: settings/aitranslation
  tasks: settings/tasks
  memory: settings/memory
  lenses: settings/lenses
  turning-it-all-off: settings/ai
---

<!-- claim:ai-intro-1 -->
Alle KI-Funktionen unten laufen über das von Ihnen gewählte Backend, und jede lässt sich
einzeln abschalten. Mit dem voreingestellten integrierten Backend verlässt kein Prompt und
keine E-Mail jemals Ihre Maschine.

## Backend wählen {#choosing-a-backend}

<!-- claim:ai-choosing-backend-1 -->
**Einstellungen → KI: Backend & Modelle** legt fest, wo die Inferenz stattfindet:

- **In der App** — eine eingebettete llama.cpp-Laufzeit. Nichts zu installieren und kein Daemon;
  sobald das Modell heruntergeladen ist, erzeugt das Antworten keinen Netzwerkverkehr. Das ist der Standard. Sie nutzt automatisch Ihre GPU, wenn
  eine vorhanden ist — Metal auf Apple Silicon, Vulkan unter Windows und Linux — und sonst die
  CPU. Auf dem Mac wird Apple Silicon (M1 oder neuer) vorausgesetzt; auf einem Intel-Mac bleibt
  sie nicht verfügbar. <!-- claim:ai-choosing-backend-2 -->
- **Ollama** — ein Ollama-Server, den Sie bereits unter `http://localhost:11434`
  betreiben. Praktisch, wenn Sie eine gemeinsame Modellbibliothek pflegen. Beachten Sie: Auf
  einem Intel-Mac erhält auch Ollama keine GPU-Beschleunigung und ist entsprechend langsam. <!-- claim:ai-choosing-backend-3 -->
- **OpenRouter** — eine kostenpflichtige Cloud-API. Erfordert einen API-Schlüssel,
  unterstützt ein monatliches Budgetlimit und sendet E-Mail-Inhalte an einen Dritten — daher
  bleibt sie aus, bis Sie sie aktivieren. Ihr Bereich zeigt die Ausgaben des laufenden
  Zeitraums gegenüber diesem Limit, mit **Neuen Zeitraum starten** zum Zurücksetzen. <!-- claim:ai-choosing-backend-4 -->

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
Ein Modell trägt die Markierung **Empfohlen**, ausgewählt für die Maschine, an der Sie sitzen:
EmailOps betrachtet den Systemspeicher und, sofern eine dedizierte Grafikkarte vorhanden ist,
auch deren Speicher, und schlägt dann das größte Modell vor, das bequem hineinpasst. Ein
Laptop und eine Workstation sehen deshalb unterschiedliche Vorschläge. Größere Modelle
antworten besser und laufen langsamer — die Markierung ist ein Ausgangspunkt, keine Regel.
Die vollständigen Anforderungen stehen unter [Installation](../installation/#with-local-ai).

### Leistungsstellschrauben {#performance-knobs}

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

## Mit dem Postfach chatten {#chat-with-your-mailbox}

<!-- claim:ai-chat-mailbox-1 -->
Fragen Sie in natürlicher Sprache — *„Was hat der Anwalt zum Vertrag gesagt?“*, *„Fasse diesen
Thread zusammen“*, *„Wer schuldet mir noch eine Antwort?“* — und erhalten Sie eine Antwort mit
Angabe der Quell-E-Mails. Die Antworten erscheinen im Stream, während sie erzeugt werden.
**In der E-Mail-Liste anzeigen** unter einer Antwort stellt genau die zitierten E-Mails in die
E-Mail-Liste, damit Sie sie öffnen und abarbeiten können.

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

<!-- claim:ai-chat-mailbox-15 -->
Geben Sie **/clear** ein und drücken Sie Enter, um eine neue Unterhaltung zu beginnen. Die
App erledigt das selbst — nichts wird an das Modell gesendet, es kostet also weder Zeit
noch Tokens.

<!-- claim:ai-chat-mailbox-10 -->
Der Chat beantwortet auch Fragen zu EmailOps selbst — *„Wie verbinde ich Ollama?“*, *„Wo
werden meine Daten gespeichert?“*, *„Was zeigt das Tag Board?“* — aus diesen Anleitungen,
in Ihrer Sprache und ohne das Postfach zu durchsuchen. Die Antwort verlinkt den verwendeten
Abschnitt, und der Link öffnet die passende Einstellung oder Ansicht. Ist eine E-Mail als
Kontext geöffnet, antwortet der Chat nur aus diesem Thread — entfernen Sie den Chip, um nach
der App zu fragen. Abschalten lässt sich das mit **Fragen zu EmailOps beantworten** unter
**Einstellungen → KI: Backend & Modelle**; dann kennt der Chat nur Ihr Postfach.

<!-- claim:ai-chat-mailbox-11 -->
Jede Antwort hat ein Panel **Begründung anzeigen**, das der Reihe nach auflistet, was
passiert ist: welche Route die Frage genommen hat und was das entschieden hat, den
Abfrageplaner, die Postfachsuche, die verwendeten Anleitungsabschnitte, jeden Modellaufruf mit
seiner Dauer und jeden Tool-Aufruf mit Argumenten und Ergebnis.

<!-- claim:ai-chat-mailbox-12 -->
Der Modus **Recherche** ist für Fragen, die alle passenden E-Mails brauchen statt der wenigen,
die eine normale Antwort liest — *„liste alle Rechnungen aus diesem Jahr“*, *„wie viele Kunden
haben ein Angebot angefragt?“*. Er schätzt zuerst, wie viele E-Mails er lesen würde und wie
lange das dauert, und fragt vor einem großen Durchlauf nach. Die Schätzung zeigt auch die ausgeführte Suche —
Absender, Zeitraum, Wörter —, damit Sie beurteilen können, ob sie sinnvoll ist, auch wenn sie
nichts gefunden hat. Dann liest er sie in Stapeln, und
Listen und Zählungen sind exakt, mit einem Link zu jeder passenden Unterhaltung. **Recherche
abbrechen** stoppt einen Durchlauf.

<!-- claim:ai-chat-mailbox-13 -->
Bitten Sie um etwas, für das die App ein Formular hat — *„lege eine Linse an, die
Lieferantenrechnungen mit Betrag und Datum verfolgt“* — und das echte Formular öffnet sich mit
ausgefüllten Feldern, damit Sie es prüfen und speichern. Der Chat legt nie selbst etwas an.

<!-- claim:ai-chat-mailbox-14 -->
**Generierung stoppen** beendet eine Antwort, während sie geschrieben wird; was schon angezeigt
wurde, bleibt erhalten.

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

## KI-Entwürfe {#ai-drafts}

<!-- claim:ai-ai-drafts-1 -->
Ein Button **KI-Entwurf** neben „Allen antworten“ schreibt eine Antwort, die im gerade
geöffneten Thread verankert ist. Konfigurieren Sie eine **Persona** (ein Satz dazu, als wer
die KI schreibt) und einen **Schreibstil** — oder ersetzen Sie die gesamte Prompt-Vorlage. Entwürfe landen im Editor, damit Sie sie vor dem Senden prüfen.

<!-- claim:ai-ai-drafts-2 -->
Ein Entwurf liest den Thread bis zu der Nachricht, auf die Sie antworten, nie die späteren
Antworten. Sie können der KI vorher sagen, was sie antworten soll, und **Neu generieren**
schreibt ihn erneut.

## Klassifizierung {#classification}

<!-- claim:ai-classification-1 -->
Jede eingehende E-Mail wird entlang dreier Achsen gekennzeichnet — **Priorität**, **Absicht**
und **Thema** — sodass sich der Posteingang praktisch selbst sortiert und die intelligenten
Filter etwas zum Filtern haben. Auch der Chat nutzt sie: Fragen Sie nach dringenden E-Mails,
listet er nur als dringend gekennzeichnete auf und sagt es, wenn es keine gibt.

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

## Semantische Suche {#semantic-search}

<!-- claim:ai-semantic-search-1 -->
E-Mails werden eingebettet, damit die Suche nach Bedeutung statt nur nach Stichwörtern
trifft — beschreiben Sie, woran Sie sich erinnern, und EmailOps findet es. Das treibt auch den Retrieval-Schritt im Chat an. Wählen Sie unter
**Einstellungen → KI-Suche**, welche Kategorien eingebettet werden, und bauen Sie den Index
nach einem Wechsel des Embedding-Modells von Grund auf neu.

<!-- claim:ai-semantic-search-2 -->
Mit den beiden lokalen Backends erfolgt das Einbetten auf Ihrem Rechner. Mit OpenRouter
geschieht es nur, wenn Sie in dessen Bereich ein **Embedding-Modell** wählen: Der Text jeder
indizierten E-Mail sowie jeder Suche und Chat-Frage wird dann an OpenRouter gesendet und zählt
zum Monatsbudget. Ohne Modell ist die semantische Suche aus und es wird nur nach Stichwörtern
gesucht. Das Modell wird beim Speichern geprüft; eines, das nicht die 768-dimensionalen
Vektoren liefert, die der Index braucht, wird abgelehnt.

## Übersetzung {#translation}

<!-- claim:ai-translation-1 -->
Bei E-Mails in einer anderen Sprache und im Verfassen-Fenster erscheinen
Übersetzen-Schaltflächen. Der Übersetzungs-Prompt ist wie die anderen bearbeitbar.

## Aufgaben {#tasks}

<!-- claim:ai-tasks-1 -->
*Experimentell.* EmailOps durchsucht E-Mails nach Handlungspunkten, Zusagen und Fristen und
sammelt sie in einem Aufgaben-Bereich. Da echte Zusagen meist in dem stehen, was **Sie**
geschrieben haben, gibt es einen Modus „nur aus selbst geschriebenen E-Mails lernen“. Sie
können Absender und Kennzeichnungen ausschließen (Newsletter sind standardmäßig
ausgeschlossen), Aufgaben pro E-Mail begrenzen, den Rückblickzeitraum einschränken und ältere
E-Mails bei Bedarf nacharbeiten lassen.

<!-- claim:ai-tasks-2 -->
Aktivieren Sie sie unter **Einstellungen → KI-Aufgaben** mit **Aufgaben aus E-Mails
extrahieren**; gefundene Aufgaben erscheinen unter **Aufgaben** in der Seitenleiste.

## Gedächtnis {#memory}

<!-- claim:ai-memory-1 -->
*Experimentell.* Fakten, die der Assistent über Ihre Kontakte, Domains und Projekte lernt,
werden als Langzeitkontext gespeichert, damit der Chat nicht jedes Mal bei null beginnt.
Kandidaten-Fakten werden bewertet und ab einem Schwellenwert übernommen; schlecht bewertete
laufen aus. Alles Gelernte ist einsehbar, und das gesamte Teilsystem hat einen Hauptschalter.

<!-- claim:ai-memory-2 -->
Aktivieren Sie es unter **Einstellungen → KI-Gedächtnis** mit **Dem Assistenten erlauben, sich
Fakten zu merken**; Gelerntes steht unter **Gedächtnis** in der Seitenleiste.

## Linsen {#lenses}

<!-- claim:ai-lenses-1 -->
Typisierte Sichten auf Ihr Postfach — gespeicherte, per KI extrahierte
strukturierte Projektionen (etwa „alle Rechnungen mit Betrag und Fälligkeit“), die Sie in der
Seitenleiste anlegen und ausführen. Eine ausgeschlossene Zeile bleibt aus der Sicht; **Ausgeschlossene
Zeilen anzeigen** holt sie zurück, sodass Sie eine wieder aufnehmen können.

<!-- claim:ai-lenses-2 -->
Linsen sind standardmäßig aktiviert: Legen Sie einzelne Linsen über den Eintrag **Linsen** in der
Seitenleiste an und führen Sie sie aus. Deaktivieren Sie sie unter **Einstellungen → KI-Linsen**.

<!-- claim:ai-lenses-3 -->
Eine Linse lässt sich auf ausgewählte **Ordner** eines Kontos beschränken, eigene IMAP-Ordner
eingeschlossen.

<!-- claim:ai-lenses-4 -->
In den Einstellungen einer Linse zeigt **Testen** vor dem Speichern, auf wie viele E-Mails die
gerade bearbeiteten Filter zutreffen, und die drei neuesten davon. Wer die Spalten oder den Prompt
ändert, markiert die extrahierten Zeilen als veraltet; das nächste **Backfill ausführen** extrahiert sie erneut
und behält die von Hand bearbeiteten Werte. Lange Werte werden in der Tabelle auf drei Zeilen
gekürzt; **mehr** zeigt den Rest.

## Skills {#skills}

<!-- claim:ai-skills-1 -->
*Experimentell.* Ein Skill ist eine gespeicherte Vorgehensweise, der der Chat bei einer Art von
Anfrage folgt – wie Ihre Wochenzusammenfassung aussehen soll, wie einem Lieferanten zu antworten
ist. Jeder Skill ist ein Ordner im Datenordner der App, `skills/<name>/SKILL.md`: ein kurzer Kopf
mit `name` (dem Ordnernamen) und einer `description`, was er tut und wann er gilt, danach die
Anweisungen in Markdown. Im Prompt des Chats stehen nur Name und Beschreibung; die Anweisungen
werden in den Runden gelesen, die den Skill nutzen. Nichts in einem Skill-Ordner wird je
ausgeführt.

<!-- claim:ai-skills-2 -->
Skills sind standardmäßig aus. Schalten Sie sie unter **Einstellungen → KI-Skills** mit
**Skills aktivieren** ein; danach erscheint **Skills** in der Seitenleiste.

<!-- claim:ai-skills-3 -->
Die Ansicht **Skills** listet jeden Skill mit eigenem Schalter und öffnet die `SKILL.md` des
gewählten in einem Editor. **Neu** legt einen Skill aus einer Vorlage an. **Speichern** schreibt
ihn zurück und lehnt Text ab, den der Chat nicht laden könnte, oder eine Datei, die ein anderer
Editor seit dem Öffnen geändert hat; wer `name` in der Datei ändert, benennt den Skill um.
**Löschen** verschiebt ihn nach `skills/.deleted`, von wo er sich von Hand wiederherstellen lässt.

<!-- claim:ai-skills-4 -->
Der Chat wendet einen Skill auf drei Arten an: Er wählt einen, dessen Beschreibung zu Ihrer Frage
passt, der Assistent lädt einen beim Antworten, oder Sie beginnen die Nachricht mit `/` und dem
Namen des Skills (`/weekly-report letzte Woche`; `/a /b` wendet beide an). Wer `/` tippt, sieht
seine aktiven Skills. Die Reasoning-Spur nennt den Skill, dem eine Runde folgte, und wer ihn
gewählt hat.

## Alles abschalten {#turning-it-all-off}

<!-- claim:ai-turning-off-1 -->
**Einstellungen → KI: Backend & Modelle → KI-Funktionen** ist ein Hauptschalter. Schalten Sie
ihn aus, und EmailOps läuft als reiner E-Mail-Client: kein Chat, keine Klassifizierung, keine
Embeddings, kein geladenes Modell. Ihre vorhandenen lokalen KI-Daten bleiben erhalten, falls
Sie ihn wieder einschalten.
