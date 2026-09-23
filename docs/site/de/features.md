---
title: 'Standardfunktionen'
description: 'Der E-Mail-Client selbst: Konten, vereinter Posteingang, Kalender, Anhänge, Suche und Junk-Filterung.'
weight: 30
nav:
  unified-inbox: view/inbox
  calendar: view/calendar
  attachments-view: view/attachments
  junk-and-bulk-mail: settings/junk
  privacy-and-security-controls: settings/privacy
  interface: settings/appearance
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
- **Tags** — komma-getrennt; sie erscheinen oben in der Ansicht als Filter-Schaltflächen. <!-- claim:feat-attachments-view-6 -->

<!-- claim:feat-attachments-view-7 -->
Alle ausgefüllten Muster müssen zutreffen. Regeln greifen bei neuer Post während der
Synchronisierung; setzen Sie **Nach dem Erstellen auf vorhandene E-Mails anwenden**, um auch aus
der bereits vorhandenen Post zu sammeln. Markieren Sie Anhänge, um sie gemeinsam in Ihren
Downloads-Ordner herunterzuladen.

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
ausdrückliches **Als Spam bestätigen**. Eine optionale Warnung vor Identitätsmissbrauch/Phishing
ist verfügbar und standardmäßig aus.

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
