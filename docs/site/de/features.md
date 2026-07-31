---
title: 'Standardfunktionen'
description: 'Der E-Mail-Client selbst: Konten, vereinter Posteingang, Kalender, Anhänge, Suche und Junk-Filterung.'
weight: 30
---

Alles auf dieser Seite funktioniert auch mit ausgeschalteter KI. Die KI-Ebene wird separat
unter [KI-Funktionen](../ai-features/) behandelt.

## Konten und Synchronisierung

Verbinden Sie beliebig viele Postfächer — Gmail, Outlook / Microsoft 365 (Graph API) und jeden
IMAP/SMTP-Server (iCloud, Yahoo, Fastmail, ProtonMail Bridge, selbst gehostet). Die E-Mails
werden in eine lokale SQLite-Datenbank synchronisiert, sodass Lesen und Suchen schnell bleiben
und offline funktionieren.

## Vereinter Posteingang

Die Ansicht **Alle Konten** führt jedes aktivierte Postfach in einer Liste zusammen, neben den
Ansichten je Konto. Eigene IMAP-Ordner werden ebenfalls synchronisiert, und Sie können sie
direkt in der App anlegen, umbenennen, löschen und Nachrichten per Drag-and-drop verschieben.

## Intelligente Filter

Grenzen Sie die Liste nach Domain, Absender oder einer Klassifizierungs-Kennzeichnung ein —
praktisch, um einen Kunden, ein Projekt oder eine Newsletter-Flut am Stück abzuarbeiten.

## Kalender

Monats-, Wochen- und Tagesansichten je Konto für Google Kalender und Outlook. Sie erhalten vor
jedem Termin eine Erinnerung mit einem Ein-Klick-Button **Teilnehmen** für Meet-, Teams-, Webex-
und Zoom-Links. Die Kalendersynchronisierung ist für Gmail- und Outlook-Konten standardmäßig
aktiv und lässt sich je Konto abschalten — ebenso die Vorlaufzeit der Benachrichtigung — unter
**Einstellungen → Kalender**.

## Anhänge-Ansicht

Ein Ort mit allen Anhängen aus Ihrem Postfach — Rechnungen, Verträge, Bilder — mit Vorschau
und Export, statt sich erneut durch Threads zu graben.

## Suche

Volltextsuche über Betreff, Inhalt, Absender und Anhänge. Mit aktivierter KI kommt die
semantische Suche hinzu, die nach Bedeutung statt nach exakten Wörtern sucht.

## Junk und Massen-E-Mails

EmailOps bewertet jede eingehende Nachricht lokal auf Spam und unerwünschte Massen-E-Mails.
Dabei ist kein Modell und kein Netzwerkaufruf beteiligt, und Ihre Korrekturen („Junk“ / „kein
Junk“) trainieren den Filter mit der Zeit. Sie entscheiden, was mit markierter Post geschieht:

- **In der Liste abschwächen** — sie bleibt vorhanden, ist für das Auge nur leicht zu
  überspringen.
- **Aus dem Posteingang nehmen** — aus der Liste entfernt, aber weiterhin über die Suche
  und die Ordner Ihres Anbieters erreichbar.

Keine der beiden Optionen verschiebt oder löscht etwas auf dem Server; das tut nur ein
ausdrückliches **Als Spam bestätigen**. Eine optionale Warnung vor Identitätsmissbrauch/Phishing
ist verfügbar und standardmäßig aus.

## Datenschutz- und Sicherheitseinstellungen

Ein Hauptpasswort sperrt die App beim Start, entfernte Bilder und Tracking-Pixel werden
blockiert, bis Sie sie erlauben, und Zugangsdaten liegen im Schlüsselbund des Systems. Alles
davon steht unter [Datenschutz und Sicherheit](../privacy-security/).

## Oberfläche

Posteingang in geteilter Ansicht oder in voller Breite, und eine Oberfläche auf Deutsch,
Englisch, Spanisch und Französisch. Die Ausgabesprache der KI wird separat eingestellt — Sie
können die Oberfläche in einer Sprache lesen und Antworten in einer anderen entwerfen lassen.
