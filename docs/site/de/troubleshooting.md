---
title: 'Fehlerbehebung'
description: 'Lösungen für die häufigsten Probleme: KI nicht verfügbar, langsamer Chat, nur Stichwortsuche, Synchronisierungsfehler.'
weight: 60
---

## KI-Funktionen sind nicht verfügbar

Prüfen Sie beim **integrierten** Backend unter **Einstellungen → KI: Backend & Modelle**, ob
das empfohlene Modell fertig heruntergeladen wurde. Ein abgebrochener Download macht das
Modell unbrauchbar — entfernen Sie es und laden Sie es erneut.

Wenn Sie zu **Ollama** gewechselt sind, stellen Sie sicher, dass der Daemon läuft und unter
`http://localhost:11434` erreichbar ist und dass Sie ein Modell geladen haben:

```bash
ollama pull llama3.2
ollama pull nomic-embed-text
```

Auf einem **Intel-Mac** ist die eingebettete Laufzeit nicht im Build enthalten. Verwenden Sie
Ollama oder OpenRouter.

## Der Chat ist langsam

Lokale Inferenz braucht echte Zeit — auf einer bescheidenen Maschine kann eine Chat-Antwort
Dutzende Sekunden dauern. Was hilft, grob nach Wirkung sortiert:

1. **Prüfen Sie, ob das Modell wirklich passt.** Das ist der große Hebel. Unter Windows oder
   Linux weicht ein Modell, das größer als der **VRAM** Ihrer GPU ist, auf die CPU aus und
   wird um ein Vielfaches langsamer — die Lösung ist ein kleineres Modell, nicht mehr
   System-RAM. Auf Apple Silicon wird mit dem gesamten Unified Memory verglichen. Siehe den
   [Modellkatalog](../ai-features/#the-model-catalog) für den Wert je Modell.
2. **Nehmen Sie ein kleineres Modell.** Qwen 3.5 4B ist nicht ohne Grund der empfohlene
   Standard.
3. **Erhöhen Sie „Modell geladen halten“** in den KI-Einstellungen, damit es nicht bei jeder
   Frage von der Festplatte neu geladen wird.
4. **Verkleinern Sie das Kontextfenster** — ein kleineres Fenster bedeutet weniger Arbeit pro
   Anfrage und ist das Erste, was man reduziert, wenn ein Modell nur knapp passt.
5. **Schalten Sie den Denkmodus aus**, der Geschwindigkeit gegen Genauigkeit tauscht.

## Die GPU wird nicht genutzt (Windows / Linux)

Das Protokoll der App nennt das Gerät, auf das ein Modell geladen wurde. Ein erfolgreicher
GPU-Ladevorgang sieht so aus:

```
llamacpp: chat model offload — Vulkan0 (Vulkan) has 15 GB free — offloading all layers
```

Fehlt eine solche Zeile, hat das Vulkan-Backend kein nutzbares Gerät gefunden und ist still
auf die CPU zurückgefallen — die App funktioniert weiter, nur langsamer. Prüfen Sie der Reihe
nach:

1. **Ihren Grafiktreiber.** Das ist fast immer die Ursache. Installieren oder aktualisieren
   Sie den normalen Treiber Ihrer Karte; ein CUDA-Toolkit oder Hersteller-SDK ist nicht nötig.
2. **Ob Vulkan das Gerät sieht.** Führen Sie `vulkaninfo --summary` aus (aus `vulkan-tools`).
   Meldet es kein Gerät, liegt das Problem unterhalb von EmailOps — bringen Sie zuerst den
   Treiberstapel in Ordnung.
3. **VRAM-Reserve.** Lagert das Protokoll nur *einige* Layer aus, ist das Modell größer als
   der freie VRAM der Karte. Wählen Sie ein kleineres Modell oder verkleinern Sie das
   Kontextfenster.

Virtuelle Maschinen und Remote-Desktops stellen häufig gar keine GPU bereit, was zu erwarten
ist.

## Die Suche liefert nur Stichwort-Treffer

Die semantische Suche braucht Embeddings. Öffnen Sie **Einstellungen → KI-Suche**, prüfen Sie,
ob die gewünschten Kategorien ausgewählt sind, und lassen Sie den Embedding-Durchlauf
abschließen. Nach einem Wechsel des Embedding-Modells bauen Sie den Index im selben Dialog neu
auf.

Prüfen Sie außerdem **KI-Verarbeitung auf neuere E-Mails beschränken** in den
KI-Einstellungen — ältere E-Mails werden
bewusst übersprungen.

## Die Klassifizierung kennzeichnet nichts

- Prüfen Sie, ob **neue E-Mails automatisch klassifizieren** unter
  **Einstellungen → KI-Klassifikation** aktiv ist.
- Sehen Sie nach, welche Gmail-Kategorien ausgewählt sind; ist keine ausgewählt, wird nichts
  klassifiziert.
- Für E-Mails, die vor dem Einschalten eintrafen, verwenden Sie **Nicht klassifizierte
  klassifizieren** oder **Alle neu klassifizieren** nach einer Änderung von Prompt oder
  Regeln.

## Die Gmail-Synchronisierung stockt oder meldet Limits

Gmail erzwingt Kontingente je Konto. Wenn es EmailOps zum Zurückhalten auffordert, pausiert
die Synchronisierung dieses Konto, bis das Zeitfenster wieder öffnet, und setzt beim nächsten
geplanten Lauf fort — Sie müssen nichts tun. Bleibt die Synchronisierung defekt, entfernen Sie
das Konto und fügen es erneut hinzu, damit ein frisches Token ausgestellt wird.

## Die App ist gesperrt und ich habe das Hauptpasswort vergessen

Das Hauptpasswort ist eine lokale Sperre ohne Wiederherstellungsweg — genau das ist der Sinn.
Ihre E-Mails liegen weiterhin auf dem Server; Sie können EmailOps gegen ein frisches
Datenverzeichnis neu installieren und erneut synchronisieren.

## Etwas anderes

Sehen Sie in die [offenen Issues](https://github.com/emailops/emailops/issues) und eröffnen
Sie ein neues, falls Ihr Problem nicht dabei ist. Nennen Sie Betriebssystem und Version, die
EmailOps-Version, welches KI-Backend und Modell Sie verwenden und was Sie erwartet hatten.
