---
title: 'Standardfunktionen'
description: 'Der E-Mail-Client selbst: Konten, vereinter Posteingang, Archiv, Zurückstellen, geplantes Senden, Signaturen, Kalender, Anhänge, Suche, Junk-Filterung, Benachrichtigungen und Tastenkombinationen.'
weight: 30
nav:
  unified-inbox: view/inbox
  calendar: view/calendar
  attachments-view: view/attachments
  junk-and-bulk-mail: settings/junk
  privacy-and-security-controls: settings/privacy
  interface: settings/appearance
  undo-send-and-scheduled-send: settings/appearance
  unsubscribe-and-block-sender: settings/junk
  keyboard-shortcuts: settings/appearance
---

<!-- claim:feat-intro-1 -->
Alles auf dieser Seite funktioniert auch mit ausgeschalteter KI. Die KI-Ebene wird separat
unter [KI-Funktionen](../ai-features/) behandelt.

## Konten und Synchronisierung

<!-- claim:feat-accounts-sync-1 -->
Verbinden Sie beliebig viele Postfächer — Gmail, Outlook / Microsoft 365 (Graph API) und jeden
IMAP/SMTP-Server (iCloud, Yahoo, Fastmail, ProtonMail Bridge, selbst gehostet). Die E-Mails
werden in eine lokale SQLite-Datenbank synchronisiert, sodass Lesen und Suchen schnell bleiben
und offline funktionieren.

<!-- claim:feat-accounts-sync-2 -->
Der Name eines Kontos ist der Absendername, den Empfänger bei den von diesem Konto
gesendeten E-Mails sehen. Sie ändern ihn im Feld **Absendername** in den Kontoeinstellungen;
bleibt es leer, wird nur mit der Adresse gesendet. Gmail-Konten übernehmen anfangs den Namen
aus der Gmail-Einstellung „Senden als“, IMAP-Konten den Anzeigenamen, den Sie beim Verbinden
angegeben haben. Über Outlook gesendete E-Mails tragen den Namen, den Microsoft für das
Postfach hinterlegt hat.

## Vereinter Posteingang {#unified-inbox}

<!-- claim:feat-unified-inbox-1 -->
Die Ansicht **Alle Konten** führt jedes aktivierte Postfach in einer Liste zusammen, neben den
Ansichten je Konto. Eigene IMAP-Ordner werden ebenfalls synchronisiert, und Sie können sie
direkt in der App anlegen, umbenennen, löschen und Nachrichten per Drag-and-drop verschieben.

## Weiterleiten

<!-- claim:reading-pane-forward -->
**Weiterleiten** steht im Lesebereich neben **Antworten** und **Allen antworten**. Der Entwurf
öffnet sich ohne Empfänger und enthält die ursprüngliche Nachricht unter der Kopfzeile
*Weitergeleitete Nachricht* mit Absender, Datum und Empfängern, dazu die ursprünglichen
Anhänge (bis zu 20 MB insgesamt). Sie wird als neue Nachricht gesendet und landet daher nicht
in bestehenden Unterhaltungen des Empfängers.

## Konversationen organisieren {#organizing-conversations}

<!-- claim:feat-organize-1 -->
**Archivieren** nimmt eine Konversation aus dem Posteingang, ohne sie zu löschen, und **In den
Posteingang verschieben** holt sie zurück. Beides, zusammen mit **Als ungelesen markieren** und
**Mit Stern markieren**, findet sich in der Leseansicht und im Menü **Weitere Aktionen** (⋮) jeder
Konversation; der Stern sitzt außerdem auf jeder Zeile der Liste. Die Änderung wird auch bei
Ihrem E-Mail-Anbieter vorgenommen, sodass Gmail oder Outlook dasselbe zeigen.

<!-- claim:feat-organize-2 -->
**Mit Stern** in der Seitenleiste listet Ihre markierten Konversationen. **Archiv** listet die
archivierte Post von Gmail- und Outlook-Konten sowie in **Alle Konten**; ein IMAP-Konto archiviert
in seinen eigenen Archivordner, der bei seinen übrigen Ordnern erscheint. Archivierte Post ist nur
aus dem Posteingang heraus: Suche, intelligente Filter und die KI-Funktionen erreichen sie
weiterhin.

<!-- claim:feat-organize-3 -->
Setzen Sie das Häkchen am Anfang einer Zeile, um sie auszuwählen. Sobald eine oder mehrere ausgewählt
sind, wirkt eine Leiste über der Liste auf alle zugleich — archivieren, zurückstellen, löschen,
als gelesen oder ungelesen markieren, mit Stern markieren — und
**Auswahl aufheben** beendet sie. Konten mit eigenen Ordnern bieten außerdem **In Ordner
verschieben**.

<!-- claim:feat-organize-4 -->
Archivieren oder Löschen nimmt die Konversationen sofort aus der Liste und zeigt 6 Sekunden lang
einen Hinweis mit **Rückgängig**. Ihr E-Mail-Anbieter erfährt erst davon, wenn diese Sekunden
vorbei sind (oder früher, wenn Sie etwas anderes archivieren oder löschen oder eine andere
Ansicht öffnen) — Rückgängig legt sie also einfach zurück.

<!-- claim:feat-organize-5 -->
Wenn die Konversation, die Sie gerade lesen, die Liste verlässt — archiviert, gelöscht,
zurückgestellt oder in den Spam verschoben —, öffnet sich die nächste. **Einstellungen →
Erscheinungsbild → Nach dem Archivieren oder Löschen** wählt zwischen der nächsten Konversation,
der vorherigen oder der Rückkehr zur Liste. Eine Konversation als ungelesen zu markieren führt
immer zurück zur Liste.

## Zurückstellen {#snooze}

<!-- claim:feat-snooze-1 -->
**Zurückstellen** blendet eine Konversation bis zu einem Zeitpunkt Ihrer Wahl aus dem Posteingang
aus: später heute, morgen, dieses Wochenende, nächste Woche oder ein selbst gewähltes Datum mit
Uhrzeit. Es steht in der Leseansicht, im ⋮-Menü der Zeile und in der Auswahlleiste zur Verfügung.

<!-- claim:feat-snooze-2 -->
Zurückgestellte Konversationen stehen unter **Zurückgestellt** in der Seitenleiste, die nächste
zuerst; **Nicht mehr zurückstellen** holt eine vorzeitig zurück. Ist der Zeitpunkt erreicht,
erscheint die Konversation wieder ganz oben im Posteingang, als ungelesen markiert. Eine neue
Nachricht in einer zurückgestellten Konversation holt sie sofort zurück.

<!-- claim:feat-snooze-3 -->
Das Zurückstellen wird nur auf diesem Computer gespeichert: andere E-Mail-Programme zeigen die
Konversation weiter im Posteingang. Konversationen kommen zurück, solange EmailOps läuft; eine,
deren Zeitpunkt bei geschlossener App verstrichen ist, kommt beim nächsten Öffnen zurück.

## Senden rückgängig machen und geplantes Senden {#undo-send-and-scheduled-send}

<!-- claim:feat-send-1 -->
Nach einem Klick auf **Senden** wartet die Nachricht ein paar Sekunden mit einem Hinweis
**Rückgängig**; Rückgängig holt sie zurück und öffnet sie wieder zum Bearbeiten. Die Wartezeit
stellen Sie unter **Einstellungen → Erscheinungsbild → Senden rückgängig machen** ein: aus, 5, 10,
20 oder 30 Sekunden, standardmäßig 10. EmailOps muss geöffnet bleiben, bis die Nachricht
verschickt ist.

<!-- claim:feat-send-2 -->
Der Pfeil neben **Senden** öffnet **Senden planen**: morgen früh, morgen Nachmittag, Montag früh
oder ein selbst gewähltes Datum mit Uhrzeit.

<!-- claim:feat-send-3 -->
Nachrichten, die noch verschickt werden sollen, stehen unter **Geplant** in der Seitenleiste, wo
Sie jede einzelne **Jetzt senden**, **Bearbeiten** oder **Löschen** können. Eine geplante Nachricht
geht nur raus, solange EmailOps geöffnet ist; eine, deren Zeitpunkt bei geschlossener App
verstrichen ist, wird beim nächsten Öffnen gesendet. Eine Nachricht, die nicht gesendet werden
konnte, bleibt dort als **Nicht gesendet** markiert, mit **Erneut versuchen**: EmailOps sendet
nie von sich aus erneut.
## Signaturen {#signatures}

<!-- claim:feat-signatures-1 -->
Jedes Konto hat seine eigene Signatur, festgelegt unter **Einstellungen → Signaturen**. Zwei
Schalter bestimmen, wo sie erscheint: **In neue Nachrichten einfügen** und **In Antworten und
Weiterleitungen einfügen**. In einer neuen Nachricht oder einer Antwort steht sie unter Ihrem
Text, in einer Weiterleitung über der weitergeleiteten Nachricht. Sie ist Teil des
Nachrichtentexts, Sie können sie also in jeder Nachricht vor dem Senden ändern oder löschen.

<!-- claim:feat-signatures-2 -->
**Bild hinzufügen** fügt ein Logo oder ein Bild Ihrer handschriftlichen Unterschrift ein. PNG,
JPEG, GIF und WebP werden angenommen; SVG und andere Dateien werden mit Begründung abgelehnt. Ein
breites Bild wird auf 600 px verkleinert, jedes Bild darf höchstens 200 KB groß sein und die ganze
Signatur 512 KB. Gmail-Konten bieten außerdem **Aus Gmail importieren**, das die Signatur, die
Gmail für diese Adresse hat, in den Editor übernimmt.

<!-- claim:feat-signatures-3 -->
In der Nur-Text-Fassung einer Nachricht steht vor einer abschließenden Signatur die übliche Zeile
`-- `, damit andere E-Mail-Programme sie erkennen. Hat das Konto eine Signatur für diese Art von
Nachricht, lassen KI-Entwürfe Ihren Namen und Ihre Kontaktdaten weg und überlassen das
Unterschreiben der Signatur.

## Intelligente Filter

<!-- claim:feat-smart-filters-1 -->
Grenzen Sie die Liste nach Domain, Absender oder einer Klassifizierungs-Kennzeichnung ein —
praktisch, um einen Kunden, ein Projekt oder eine Newsletter-Flut am Stück abzuarbeiten. Mit
aktivierter KI speisen dieselben Kennzeichnungen auch das [Tag-Board](../ai-features/#tag-board),
das sie als Raster aus Blöcken darstellt.

## Kalender {#calendar}

<!-- claim:feat-calendar-1 -->
Monats-, Wochen- und Tagesansichten je Konto für Google Kalender und Outlook. Sie erhalten vor
jedem Termin eine Erinnerung mit einem Ein-Klick-Button **Teilnehmen** für Meet-, Teams-, Webex-
und Zoom-Links. Die Kalendersynchronisierung ist für Gmail- und Outlook-Konten standardmäßig
aktiv und lässt sich je Konto abschalten — ebenso die Vorlaufzeit der Benachrichtigung — unter
**Einstellungen → Kalender**.

<!-- claim:feat-calendar-2 -->
Synchronisiert werden alle Kalender eines Kontos, nicht nur der primäre — ein Kalender, den
eine Kollegin mit Ihnen geteilt hat, erscheint hier also genauso wie in Google oder Outlook.
Jeder erhält die Farbe, die sein Anbieter vergibt, und die Legende über dem Raster blendet
einzelne Kalender aus oder ein; dieselben Schalter finden sich unter
**Einstellungen → Kalender**.

## Anhänge-Ansicht {#attachments-view}

<!-- claim:feat-attachments-view-1 -->
Ein Ort für die Anhänge, die Ihnen wichtig sind — Rechnungen, Verträge, Belege — mit Vorschau
und Download, statt sich erneut durch Threads zu graben. Öffnen Sie sie über **Anhänge** in der
Seitenleiste.

<!-- claim:feat-attachments-view-2 -->
Die Ansicht sammelt Anhänge über **Regeln** und ist daher anfangs leer. Klicken Sie auf **Regeln
verwalten** (oder **Regel erstellen** in der leeren Ansicht) und füllen Sie aus:

- **Regelname** — so erscheint die Regel in der Liste. <!-- claim:feat-attachments-view-3 -->
- **Absender-Muster** — komma-getrennt; exakter Treffer, sofern es kein `*` enthält
  (`*apple.com*` erfasst jeden Absender, der „apple.com“ enthält). Leer lassen für jeden Absender. <!-- claim:feat-attachments-view-4 -->
- **Betreffmuster** und **Dateinamen-Muster** — `*` ist ein Platzhalter; nur passende Dateinamen
  werden gesammelt. <!-- claim:feat-attachments-view-5 -->
- **Tags** — wählen Sie vorhandene Tags oder tippen Sie, um einen neuen anzulegen; sie erscheinen oben in der Ansicht als Filter-Schaltflächen. <!-- claim:feat-attachments-view-6 -->

<!-- claim:feat-attachments-view-7 -->
Alle ausgefüllten Muster müssen zutreffen. Regeln greifen bei neuer Post während der
Synchronisierung; setzen Sie **Nach dem Erstellen auf vorhandene E-Mails anwenden**, um auch aus
der bereits vorhandenen Post zu sammeln. Regeln erfassen den Posteingang, Gesendete Elemente, das
Archiv und Ihre eigenen Ordner, nie Spam oder Papierkorb. Markieren Sie Anhänge, um sie gemeinsam in Ihren
Downloads-Ordner herunterzuladen.

<!-- claim:feat-attachments-view-8 -->
EmailOps schlägt auch selbst Regeln vor. Wenn Ihnen derselbe Absender immer wieder Dokumente
schickt (PDFs, Office-Dateien oder E-Rechnungen) — mindestens zwei E-Mails in zwei
verschiedenen Monaten, im Posteingang oder in einem Ordner, in dem Sie sie ablegen —, erscheint unter **Regeln verwalten** ein Abschnitt **Vorgeschlagene Regeln**, und ein
Zähler neben **Anhänge** in der Seitenleiste zeigt ihre Anzahl. **Prüfen** öffnet das
Regelformular bereits ausgefüllt (Absender und Dateinamenmuster); die Regel wird erst angelegt,
wenn Sie sie speichern. **Verwerfen** blendet den Vorschlag dauerhaft aus, auch wenn der
Absender später von einer anderen Adresse schreibt; **Rückgängig** oder **Wiederherstellen** unter
**Verworfene Vorschläge** holt ihn zurück. E-Mails von Ihrer eigenen Adresse oder von
Kollegen Ihres eigenen Unternehmens werden nie vorgeschlagen.

## EO Docs {#eo-docs}

<!-- claim:feat-eo-docs-1 -->
Mit **EO Docs** schreiben Sie Dokumente und Tabellen gemeinsam mit anderen EmailOps-Nutzern, ohne
Cloud dazwischen: Jede Änderung reist als gewöhnliche E-Mail zwischen Ihren Konten, und jede Kopie
führt Eingehendes ohne Konflikte zusammen. Die Funktion ist experimentell und standardmäßig an;
**Einstellungen → EO Docs** schaltet sie ab, und solange sie aus ist, wird nichts empfangen oder gesendet.

<!-- claim:feat-eo-docs-2 -->
Öffnen Sie **EO Docs** in der Seitenleiste und klicken Sie auf **Neu**, um ein Dokument oder eine
Tabelle anzulegen. Dokumente kennen Überschriften, Fett, Kursiv, Unterstreichen, Listen, Links,
Tabellen und Bilder. Tabellen wachsen um Zeilen und Spalten, übernehmen einen aus Excel eingefügten
Block, und eine Spalte wird breiter, wenn Sie den Rand ihres Kopfes ziehen. Rückgängig und
Wiederholen betreffen nur Ihre eigenen Änderungen.

<!-- claim:feat-eo-docs-3 -->
Eine Zelle, die mit `=` beginnt, ist eine Formel: `SUM`, `AVERAGE`, `MIN`, `MAX` und `COUNT`
(oder `SUMA`, `PROMEDIO` und `CONTAR`) über Bereiche wie `=SUM(B2:B10)`. Beim Einfügen oder Löschen
von Zeilen zeigen die Bereiche weiter auf dieselben Zellen. Die Filterschaltfläche im Spaltenkopf
blendet die Zeilen aus, deren Haken Sie entfernen; Filter ändern nur Ihre eigene Ansicht.

<!-- claim:feat-eo-docs-9 -->
Tabellen sind vorerst einfach gehalten: Text, Zahlen und diese Formeln, noch ohne Formatierung,
Diagramme oder weitere Funktionen. Ein Hinweis über jeder Tabelle sagt das, bis Sie ihn schließen.

<!-- claim:feat-eo-docs-8 -->
Ändern zwei Personen dieselbe Zelle, bevor sie die Änderung der anderen gesehen haben, behalten
alle Kopien denselben der beiden Werte. Die Zelle wird dann markiert, ein Hinweis nennt den
verlorenen Wert, und Sie entscheiden: ihn zurückholen oder den angezeigten behalten. **Verlauf**
listet diese Zellen ebenfalls auf, geklärt oder nicht.

<!-- claim:feat-eo-docs-4 -->
**Teilen** fragt nach den E-Mail-Adressen, schlägt zuerst Kollegen aus Ihrem Unternehmen vor, und
nach Ihrer Zustimmung: Von da an mailt EmailOps Ihre Änderungen von selbst, etwa zwei Minuten nachdem
Sie aufgehört haben zu tippen, oder sofort mit **Änderungen jetzt senden**. Andere EmailOps-Nutzer
erhalten eine Einladung zum **Annehmen**; alle anderen erhalten in der Einladung eine
schreibgeschützte Kopie. Änderungen kommen mit der nächsten Synchronisierung, werden zusammengeführt,
und ihre E-Mails werden als gelesen markiert und archiviert.

<!-- claim:feat-eo-docs-5 -->
Eine Änderung wird nur übernommen, wenn sie von jemandem kommt, mit dem das Dokument geteilt ist. Bei
Gmail- und Outlook-Konten wird sie außerdem abgelehnt, wenn der Absender die
Authentifizierungsprüfung Ihres Anbieters nicht besteht (DMARC, oder SPF ohne gültige
DKIM-Signatur). Die E-Mails sind nicht Ende-zu-Ende-verschlüsselt: Sie sind so privat wie Ihre übrige Post.

<!-- claim:feat-eo-docs-6 -->
Eigene Ordner (nie geteilt) halten die Dokumente in Ordnung; ziehen Sie ein Dokument auf einen
Ordner, oder nutzen Sie **Verschieben nach**. Die Suche findet Dokumente nach Titel und Inhalt, und
**Verlauf** zeigt frühere Versionen. **Als PDF exportieren** speichert das Dokument als PDF in Ihrem
Downloads-Ordner. **Löschen** fragt vorher nach; ein geteiltes Dokument zu löschen entfernt
nur Ihre Kopie, die anderen behalten ihre.

<!-- claim:feat-eo-docs-7 -->
**Importieren** macht aus einem Word-Dokument (`.docx`) oder einer Tabelle (`.xlsx`, `.xls`, `.ods`)
EO Docs, eine Tabelle pro Blatt und Formeln als ihre Werte; **In EO Docs öffnen** tut dasselbe mit
einem E-Mail-Anhang. Im E-Mail-Editor hängt **Aus EO Docs** ein Dokument an und teilt es damit mit
den Empfängern der E-Mail.

## Suche

<!-- claim:feat-search-1 -->
Volltextsuche über Betreff, Inhalt, Absender und Anhänge. Mit aktivierter KI kommt die
semantische Suche hinzu, die nach Bedeutung statt nach exakten Wörtern sucht.

<!-- claim:feat-search-2 -->
Suchen lassen sich mit Operatoren eingrenzen, allein oder neben freiem Text:

| Operator | Sucht nach |
|---|---|
| `from:ana` | Absenderadresse oder -name |
| `to:ana` | Empfänger |
| `subject:rechnung` | Betreff |
| `before:2026-09-01` / `after:2026-09-01` | Empfangsdatum |
| `id:<E-Mail-ID>` | eine bestimmte E-Mail |
| `tag:newsletter` / `tag:intent=request` | ein Tag der Klassifizierung, optional innerhalb einer Facette |

## Junk und Massen-E-Mails {#junk-and-bulk-mail}

<!-- claim:feat-junk-bulk-1 -->
EmailOps bewertet jede eingehende Nachricht lokal auf Spam und unerwünschte Massen-E-Mails.
Dabei ist kein Modell und kein Netzwerkaufruf beteiligt, und Ihre Korrekturen („Junk“ / „kein
Junk“) trainieren den Filter mit der Zeit. Sie entscheiden, was mit markierter Post geschieht:

- **In der Liste abschwächen** — sie bleibt vorhanden, ist für das Auge nur leicht zu
  überspringen. <!-- claim:feat-junk-bulk-2 -->
- **Aus dem Posteingang nehmen** — aus der Liste entfernt, aber weiterhin über die Suche
  und die Ordner Ihres Anbieters erreichbar. <!-- claim:feat-junk-bulk-3 -->

<!-- claim:feat-junk-bulk-4 -->
Keine der beiden Optionen verschiebt oder löscht etwas auf dem Server; das tut nur ein
ausdrückliches **Als Spam bestätigen** oder **Absender blockieren**. Eine optionale Warnung vor Identitätsmissbrauch/Phishing
ist verfügbar und standardmäßig aus.

## Abbestellen und Absender blockieren {#unsubscribe-and-block-sender}

<!-- claim:feat-unsubscribe-1 -->
Ein Newsletter oder eine Nachricht einer Mailingliste, die angibt, wie man sich abmeldet, zeigt
**Abbestellen** neben dem Absender. Bevor etwas verschickt wird, erklärt eine Bestätigung genau,
was passiert: eine Anfrage direkt an den Server des Absenders (nicht über Ihren E-Mail-Anbieter),
eine Abmelde-E-Mail von Ihrem Konto oder die Seite des Absenders in Ihrem Browser.

<!-- claim:feat-block-sender-1 -->
**Absender blockieren** im ⋮-Menü einer Konversation schickt neue Post dieses Absenders in diesem
Konto in den Spam und meldet sie Ihrem E-Mail-Anbieter als Spam. **Vorhandene Nachrichten
ebenfalls in den Spam verschieben** räumt auf, was schon da ist, und die Konversation zeigt an,
dass der Absender blockiert ist, mit **Blockierung aufheben** gleich zur Hand.

<!-- claim:feat-block-sender-2 -->
**Einstellungen → Spam → Blockierte Absender** listet alle, die Sie blockiert haben, mit
**Blockierung aufheben**, das ihre Nachrichten auch aus dem Spam zurück in den Posteingang holen
kann. **Aus den intelligenten Filtern ausblenden** im ⋮-Menü ist etwas anderes: Es entfernt den
Absender nur aus den intelligenten Filtern der Seitenleiste.

## Benachrichtigungen für neue E-Mails {#new-mail-notifications}

<!-- claim:feat-notifications-1 -->
EmailOps zeigt eine Desktop-Benachrichtigung, wenn neue Post in Ihrem Posteingang ankommt und
wenn eine zurückgestellte Konversation zurückkehrt — nie bei der ersten Synchronisierung eines
Kontos, für ältere Post, bereits gelesene Post, Spam oder blockierte Absender. Mehr als drei neue
Nachrichten auf einmal werden zu einer einzigen Zusammenfassung. Ein Klick auf eine
Benachrichtigung holt EmailOps in den Vordergrund; die Nachricht öffnet er nicht.

<!-- claim:feat-notifications-2 -->
**Einstellungen → Benachrichtigungen** enthält den Hauptschalter, einen Schalter pro Konto, den
**Inhalt der Benachrichtigung** (Absender und Betreff oder **Inhalt ausblenden**, das nur das Konto
zeigt) und **Nur wenn EmailOps nicht im Vordergrund ist**; alles ist standardmäßig an, mit
sichtbarem Absender und Betreff. Der Nachrichtentext wird nie gezeigt, und solange die App mit dem
Hauptpasswort gesperrt ist, auch Absender und Betreff nicht.

## Datenschutz- und Sicherheitseinstellungen {#privacy-and-security-controls}

<!-- claim:feat-privacy-security-1 -->
Ein Hauptpasswort sperrt die App beim Start, entfernte Bilder und Tracking-Pixel werden
blockiert, bis Sie sie erlauben, und Zugangsdaten liegen im Schlüsselbund des Systems. Alles
davon steht unter [Datenschutz und Sicherheit](../privacy-security/).

## Oberfläche {#interface}

<!-- claim:feat-interface-1 -->
Posteingang in geteilter Ansicht oder in voller Breite, und eine Oberfläche auf Deutsch,
Englisch, Spanisch und Französisch. Die Ausgabesprache der KI wird separat eingestellt — Sie
können die Oberfläche in einer Sprache lesen und Antworten in einer anderen entwerfen lassen.

## Tastenkombinationen {#keyboard-shortcuts}

<!-- claim:feat-shortcuts-1 -->
Drücken Sie `?` irgendwo außerhalb eines Textfelds, um alle Tastenkombinationen zu sehen. Sie folgen
denen von Gmail:

| Tasten | Aktion |
|---|---|
| `j` / `k` | nächste / vorherige Konversation |
| `Enter` oder `o`, `u` | Konversation öffnen, zurück zur Liste |
| `x` | Konversation auswählen oder abwählen |
| `e`, `#`, `s`, `b` | archivieren, löschen, mit Stern markieren, zurückstellen |
| `Shift+U` / `Shift+I` | als ungelesen / gelesen markieren |
| `c`, `r`, `a`, `f` | neue Nachricht, antworten, allen antworten, weiterleiten |
| `g`, dann `i`, `s`, `b`, `a`, `l` | zu Posteingang, Mit Stern, Zurückgestellt, Archiv, Geplant |
| `/` | suchen |

<!-- claim:feat-shortcuts-2 -->
Die Tastenkombinationen pausieren, während Sie tippen und solange ein Dialog oder Menü offen ist.
Die Schaltflächen, für die sie stehen, nennen ihre Taste im Tooltip, etwa „Archivieren (E)".
**Einstellungen → Erscheinungsbild → Tastenkombinationen** schaltet sie ab, und **Liste anzeigen**
öffnet dieselbe Übersicht wie `?`.
