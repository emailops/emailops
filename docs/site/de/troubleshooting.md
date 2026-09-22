---
title: 'Fehlerbehebung'
description: 'Lösungen für die häufigsten Probleme: KI nicht verfügbar, langsamer Chat, nur Stichwortsuche, Synchronisierungsfehler.'
weight: 60
---

## KI-Funktionen sind nicht verfügbar

<!-- claim:trbl-ai-features-1 -->
Prüfen Sie beim **integrierten** Backend unter **Einstellungen → KI: Backend & Modelle**, ob
das empfohlene Modell fertig heruntergeladen wurde. Ein abgebrochener Download wird nie
als Modell verwendet: Starten Sie ihn im selben Bildschirm erneut, dann setzt er dort fort, wo er
aufgehört hat; ein Download, der die Prüfung nicht besteht, wird automatisch verworfen.

<!-- claim:trbl-ai-features-2 -->
Wenn Sie zu **Ollama** gewechselt sind, stellen Sie sicher, dass der Daemon läuft und unter
`http://localhost:11434` erreichbar ist und dass Sie ein Modell geladen haben:

```bash
ollama pull llama3.2
ollama pull nomic-embed-text
```

<!-- claim:trbl-ai-features-3 -->
Auf einem **Intel-Mac** kann die eingebaute KI grundsätzlich nicht laufen — sie benötigt einen
Apple-Silicon-Chip (M1 oder neuer), daher lässt EmailOps sie deaktiviert. Verwenden Sie
stattdessen OpenRouter. Ollama lässt sich zwar installieren, erhält auf Intel aber ebenfalls
keine GPU-Beschleunigung und ist damit zu langsam, um Freude zu machen.

## Der Chat ist langsam

<!-- claim:trbl-chat-slow-1 -->
Lokale Inferenz braucht echte Zeit — auf einer bescheidenen Maschine kann eine Chat-Antwort
Dutzende Sekunden dauern. Was hilft, grob nach Wirkung sortiert:

1. **Prüfen Sie, ob das Modell wirklich passt.** Das ist der große Hebel. Unter Windows oder
   Linux weicht ein Modell, das größer als der **VRAM** Ihrer GPU ist, auf die CPU aus und
   wird um ein Vielfaches langsamer — die Lösung ist ein kleineres Modell, nicht mehr
   System-RAM. Auf Apple Silicon wird mit dem gesamten Unified Memory verglichen. Siehe den
   [Modellkatalog](../ai-features/#the-model-catalog) für den Wert je Modell. <!-- claim:trbl-chat-slow-2 -->
2. **Nehmen Sie ein kleineres Modell.** Qwen 3.5 4B ist das kleinste Chat-Modell im
   Katalog. <!-- claim:trbl-chat-slow-3 -->
3. **Erhöhen Sie „Modell geladen halten“** in den KI-Einstellungen, damit es nicht bei jeder
   Frage von der Festplatte neu geladen wird. <!-- claim:trbl-chat-slow-4 -->
4. **Verkleinern Sie das Kontextfenster** — ein kleineres Fenster bedeutet weniger Arbeit pro
   Anfrage und ist das Erste, was man reduziert, wenn ein Modell nur knapp passt. <!-- claim:trbl-chat-slow-5 -->
5. **Schalten Sie den Denkmodus aus**, der Geschwindigkeit gegen Genauigkeit tauscht. <!-- claim:trbl-chat-slow-6 -->

## Die GPU wird nicht genutzt (Windows / Linux)

<!-- claim:trbl-gpu-used-1 -->
Das Protokoll der App nennt das Gerät, auf das ein Modell geladen wurde. Ein erfolgreicher
GPU-Ladevorgang sieht so aus:

```
llamacpp: chat model offload — Vulkan0 (Vulkan) has 15 GB free — offloading all layers
```

<!-- claim:trbl-gpu-used-2 -->
Fehlt eine solche Zeile, hat das Vulkan-Backend kein nutzbares Gerät gefunden und ist still
auf die CPU zurückgefallen — die App funktioniert weiter, nur langsamer. Prüfen Sie der Reihe
nach:

1. **Ihren Grafiktreiber.** Das ist fast immer die Ursache. Installieren oder aktualisieren
   Sie den normalen Treiber Ihrer Karte; ein CUDA-Toolkit oder Hersteller-SDK ist nicht nötig. <!-- claim:trbl-gpu-used-3 -->
2. **Ob Vulkan das Gerät sieht.** Führen Sie `vulkaninfo --summary` aus (aus `vulkan-tools`).
   Meldet es kein Gerät, liegt das Problem unterhalb von EmailOps — bringen Sie zuerst den
   Treiberstapel in Ordnung. <!-- claim:trbl-gpu-used-4 -->
3. **VRAM-Reserve.** Lagert das Protokoll nur *einige* Layer aus, ist das Modell größer als
   der freie VRAM der Karte. Wählen Sie ein kleineres Modell oder verkleinern Sie das
   Kontextfenster. <!-- claim:trbl-gpu-used-5 -->

<!-- claim:trbl-gpu-used-6 -->
Virtuelle Maschinen und Remote-Desktops stellen häufig gar keine GPU bereit, was zu erwarten
ist.

## Die Suche liefert nur Stichwort-Treffer

<!-- claim:trbl-search-returns-1 -->
Die semantische Suche braucht Embeddings. Öffnen Sie **Einstellungen → KI-Suche**, prüfen Sie,
ob die gewünschten Kategorien ausgewählt sind, und lassen Sie den Embedding-Durchlauf
abschließen. Nach einem Wechsel des Embedding-Modells bauen Sie den Index im selben Dialog neu
auf.

<!-- claim:trbl-search-returns-2 -->
Prüfen Sie außerdem **KI-Verarbeitung begrenzen** in den
KI-Einstellungen — ältere E-Mails werden
bewusst übersprungen.

## Die Klassifizierung kennzeichnet nichts

- Prüfen Sie, ob **neue E-Mails automatisch klassifizieren** unter
  **Einstellungen → KI-Klassifikation** aktiv ist. <!-- claim:trbl-classification-tagging-1 -->
- Sehen Sie nach, welche Gmail-Kategorien ausgewählt sind; ist keine ausgewählt, wird nichts
  klassifiziert. <!-- claim:trbl-classification-tagging-2 -->
- Für E-Mails, die vor dem Einschalten eintrafen, verwenden Sie **Nicht klassifizierte
  klassifizieren** oder **Alle neu klassifizieren** nach einer Änderung von Prompt oder
  Regeln. <!-- claim:trbl-classification-tagging-3 -->

## Die Gmail-Synchronisierung stockt oder meldet Limits

<!-- claim:trbl-gmail-sync-1 -->
Gmail erzwingt Kontingente je Konto. Wenn es EmailOps zum Zurückhalten auffordert, pausiert
die Synchronisierung dieses Konto, bis das Zeitfenster wieder öffnet, und setzt beim nächsten
geplanten Lauf fort — Sie müssen nichts tun. Bleibt die Synchronisierung defekt, entfernen Sie
das Konto und fügen es erneut hinzu, damit ein frisches Token ausgestellt wird.

## Die App ist gesperrt und ich habe das Hauptpasswort vergessen

<!-- claim:trbl-app-locked-1 -->
Das Hauptpasswort ist eine lokale Sperre ohne Wiederherstellungsweg — genau das ist der Sinn.
Ihre E-Mails liegen weiterhin auf dem Server; Sie können EmailOps gegen ein frisches
Datenverzeichnis neu installieren und erneut synchronisieren.

## Etwas anderes

<!-- claim:trbl-something-else-1 -->
Sehen Sie in die [offenen Issues](https://github.com/emailops/emailops/issues) und eröffnen
Sie ein neues, falls Ihr Problem nicht dabei ist. Nennen Sie Betriebssystem und Version, die
EmailOps-Version, welches KI-Backend und Modell Sie verwenden und was Sie erwartet hatten.
