---
title: 'Installation'
description: 'EmailOps unter macOS, Windows oder Linux herunterladen und installieren.'
weight: 10
---

## Systemanforderungen

Die KI **lokal** auszuführen ist der Teil, der leistungsfähigere Hardware erfordert — und
optional. Sie können sie im Einrichtungsassistenten deaktivieren und EmailOps als reinen
E-Mail-Client nutzen, oder alle KI-Funktionen behalten und sie stattdessen an einen
entfernten Anbieter leiten. Die beiden Modi haben sehr unterschiedliche Anforderungen.

### Mit lokaler KI {#with-local-ai}

Eine der wichtigsten Voraussetzungen für das lokale Ausführen von KI ist der verfügbare
Speicher zum Laden des Modells und seines Kontexts. Je nach Maschine handelt es sich dabei
um die eine oder die andere Art von Speicher:

| | Apple-Silicon-Mac | Windows / Linux |
|---|---|---|
| Führt das Modell aus auf | Der eingebauten GPU, über Metal | Ihrer GPU, über Vulkan — oder der CPU, wenn keine GPU vorhanden ist |
| Speicher, in den es passen muss | Unified Memory, mit dem System geteilt | Der **VRAM** der GPU, oder System-RAM auf der CPU |
| Minimum | 8 GB unified | 8 GB VRAM, oder 16 GB System-RAM ohne GPU |
| Empfohlen | 16 GB unified oder mehr | 12–16 GB VRAM |
| Freier Speicherplatz | ~3 GB — App plus Standard-Chat-Modell | ~3 GB — App plus Standard-Chat-Modell |

**Faustregel:** Das Modell muss vollständig in den Speicher passen, in dem es läuft. Das
voreingestellte **Qwen 3.5 4B** braucht rund 8 GB; das größte Modell im Katalog verlangt
32 GB. Den Wert für jedes Modell finden Sie im
[Modellkatalog](../ai-features/#the-model-catalog).

- **Apple Silicon** hat Unified Memory — die GPU adressiert denselben Pool wie die CPU, zu
  vergleichen ist also der gesamte Systemspeicher. Ein 16-GB-Mac führt Modelle bis zur
  16-GB-Zeile bequem aus, abzüglich dessen, was macOS und Ihre anderen Apps bereits belegen.
- **Eine GPU unter Windows oder Linux** hat eigenen, getrennten VRAM, und das ist die Zahl,
  die zählt — 32 GB System-RAM helfen nicht, wenn die Karte nur 8 GB hat. Ein Modell, das
  nicht hineinpasst, weicht auf die CPU aus: das funktioniert, ist aber um ein Vielfaches
  langsamer.
- **Ganz ohne GPU** wird unterstützt und erfordert keinen anderen Download. Die App fällt auf
  CPU und System-RAM zurück; rechnen Sie den Modellwert im RAM ein und mit spürbar längeren
  Antwortzeiten.

Intel-Macs sind die Ausnahme: Die eingebettete KI-Laufzeit benötigt einen Apple-Silicon-Chip
(M1 oder neuer) und läuft dort gar nicht — siehe den [Hinweis unten](#direct-download).

### Ohne lokale KI

| | Minimum | Empfohlen |
|---|---|---|
| RAM | 2 GB | 4 GB |
| Freier Speicherplatz | ~500 MB, plus Platz für die synchronisierten E-Mails | Je nach Postfachgröße |
| Prozessor | 64 Bit, 2 Kerne | — |
| Grafik | Keine | Keine |

Das sind die Anforderungen in zwei Fällen: KI komplett aus, und KI eingeschaltet, **aber an
OpenRouter geleitet**. Entfernte Inferenz läuft auf fremder Hardware, ein altes Notebook
genügt also — um den Preis eines API-Schlüssels, nutzungsabhängiger Kosten und Ihrer
E-Mail-Inhalte, die das Gerät verlassen. Siehe
[Backend wählen](../ai-features/#choosing-a-backend).

### Betriebssystem

Beide Modi benötigen eines davon:

- **macOS** Monterey (12) oder neuer — Apple Silicon oder Intel.
- **Windows** 10 oder 11, 64 Bit.
- **Linux** 64 Bit, mit WebKitGTK und einem Secret-Service-Schlüsselbund — siehe
  [Linux](#linux) unten.

## macOS

### Homebrew

```bash
brew install --cask emailops/tap/emailops
```

Später aktualisieren mit `brew upgrade --cask emailops`.

### Direkter Download {#direct-download}

1. Laden Sie **EmailOps-macos.dmg** aus der
   [neuesten Version](https://github.com/emailops/emailops/releases/latest) herunter.
2. Öffnen Sie das DMG und ziehen Sie **EmailOps.app** in Ihren Programme-Ordner.
3. Starten Sie es aus dem Programme-Ordner.

> **Intel-Macs:** Dieser eine Download funktioniert auf jedem Mac — es gibt keinen separaten
> Intel-Build. Die Ausnahme sind die KI-Funktionen: Die eingebaute KI benötigt einen
> Apple-Silicon-Chip (M1 oder neuer). Auf einem Intel-Mac bleibt sie deaktiviert, und EmailOps
> erklärt Ihnen warum. Alles andere funktioniert normal. Für KI richten Sie EmailOps auf
> [OpenRouter](../ai-features/#choosing-a-backend) aus.

## Windows

1. Laden Sie **EmailOps-windows.exe** aus der
   [neuesten Version](https://github.com/emailops/emailops/releases/latest) herunter.
2. Führen Sie das Installationsprogramm aus und folgen Sie den Schritten.
3. Starten Sie EmailOps über das Startmenü.

### GPU-Beschleunigung

Es ist nichts zusätzlich zu installieren. Die Windows-Version enthält ein **Vulkan**-Backend,
das zur Laufzeit geladen wird, sobald ein funktionierender Grafiktreiber vorhanden ist, und
sonst auf die CPU zurückfällt — in beiden Fällen derselbe Download.

Vulkan wurde genau deshalb CUDA vorgezogen, damit es einfach bleibt: Es deckt AMD, Intel und
NVIDIA über den Grafiktreiber ab, den Sie ohnehin haben, ohne Herstellerpaket zur
Installation. Halten Sie Ihren GPU-Treiber halbwegs aktuell, dann funktioniert es.

## Linux {#linux}

1. Laden Sie **EmailOps-linux.AppImage** aus der
   [neuesten Version](https://github.com/emailops/emailops/releases/latest) herunter.
2. Machen Sie sie ausführbar und starten Sie sie:

```bash
chmod +x EmailOps-linux.AppImage
./EmailOps-linux.AppImage
```

### GPU-Beschleunigung

Wie unter Windows: Die AppImage enthält ein **Vulkan**-Backend, das automatisch verwendet
wird, wenn ein Grafiktreiber vorhanden ist, und sonst auf die CPU zurückfällt. Kein
CUDA-Toolkit, kein Hersteller-SDK, kein separater Build zur Auswahl.

Was Sie brauchen, ist der gewöhnliche Vulkan-Treiberstapel für Ihre Karte —
`mesa-vulkan-drivers` bei AMD und Intel, der proprietäre NVIDIA-Treiber bei NVIDIA — den die
meisten Desktop-Distributionen bereits installieren. Wenn `vulkaninfo` ein Gerät meldet, nutzt
EmailOps es.

### Ein Schlüsselbund ist erforderlich

EmailOps schreibt Zugangsdaten nie in eine Datei — OAuth-Tokens und IMAP-Passwörter gehen in
den Anmeldeinformationsspeicher des Systems. macOS und Windows bringen einen mit
(Schlüsselbund und Anmeldeinformationsverwaltung); unter Linux müssen Sie selbst einen
bereitstellen.

Sie brauchen einen installierten und entsperrten **Secret-Service**-Anbieter. Jeder davon
funktioniert:

- **GNOME Keyring** (`gnome-keyring`) — Standard unter GNOME, Ubuntu, Fedora Workstation.
- **KWallet** (`kwalletmanager` mit Secret-Service-Schnittstelle) — das KDE-Gegenstück.
- **KeePassXC** mit aktivierter *Einstellungen → Secret-Service-Integration*.

Auf einem minimalen Fenstermanager oder in einer Sitzung ohne Desktop läuft oft gar kein
Schlüsselbund. Installieren Sie einen der obigen und stellen Sie sicher, dass er beim Start
von EmailOps entsperrt ist — sonst schlägt das Hinzufügen eines Kontos fehl, weil es keinen
sicheren Ort für die Zugangsdaten gibt.

```bash
# Debian / Ubuntu
sudo apt install gnome-keyring

# Fedora
sudo dnf install gnome-keyring

# Arch
sudo pacman -S gnome-keyring
```

## Wo Ihre Daten liegen

Alles, was EmailOps speichert, liegt auf Ihrer Maschine, im Anwendungsdatenverzeichnis Ihres
Betriebssystems:

- **E-Mails, Kontakte, Kalendertermine, Embeddings** — eine lokale SQLite-Datenbank.
- **Heruntergeladene KI-Modelle** — ein Ordner `models/` neben der Datenbank.
- **OAuth-Tokens und Passwörter** — der Schlüsselbund Ihres Systems, nie eine Klartextdatei.

Um ein Datenverzeichnis zu verschieben oder zu teilen (für Tests oder ein zweites Profil),
setzen Sie vor dem Start die Umgebungsvariable `EMAILOPS_DATA_DIR`. Die genauen Pfade je
Plattform und was wo geschrieben wird, stehen unter
[Datenschutz und Sicherheit](../privacy-security/#where-your-data-is-stored).

## Deinstallieren

Beim Entfernen der App bleiben Ihre E-Mail-Datenbank und die heruntergeladenen Modelle
absichtlich erhalten, damit eine Neuinstallation dort weitermacht, wo Sie aufgehört haben.
Löschen Sie zusätzlich das Datenverzeichnis, wenn Sie ganz von vorn anfangen wollen.

In keinem Fall wird etwas bei Ihrem E-Mail-Anbieter gelöscht — das Deinstallieren von EmailOps
rührt die E-Mails bei Gmail, Outlook oder Ihrem IMAP-Server nie an.

### macOS

Mit Homebrew entfernt ein Befehl die App und ihre Daten:

```bash
brew uninstall --zap --cask emailops
```

Ohne `--zap` verschwindet nur die App. Von Hand: Ziehen Sie **EmailOps.app** aus dem
Programme-Ordner in den Papierkorb und löschen Sie dann:

```
~/Library/Application Support/com.emailops.app
~/Library/Caches/com.emailops.app
~/Library/HTTPStorages/com.emailops.app
~/Library/Preferences/com.emailops.app.plist
~/Library/Saved Application State/com.emailops.app.savedState
~/Library/WebKit/com.emailops.app
```

### Windows

Deinstallieren Sie über **Einstellungen → Apps → Installierte Apps → EmailOps**, oder starten
Sie das Deinstallationsprogramm über den Startmenü-Eintrag. Löschen Sie anschließend das
Datenverzeichnis:

```
%APPDATA%\com.emailops.app
```

### Linux

Löschen Sie die AppImage-Datei. Löschen Sie danach die Daten- und Konfigurationsverzeichnisse:

```bash
rm -rf ~/.local/share/com.emailops.app
rm -rf ~/.config/com.emailops.app
```

### Gespeicherte Zugangsdaten

Auf allen Plattformen liegen OAuth-Tokens und IMAP-Passwörter im Schlüsselbund des Systems und
nicht im Datenverzeichnis — sie überstehen also alles Obige. Entfernen Sie die Einträge
`com.emailops.app` aus der Schlüsselbundverwaltung (macOS), der Anmeldeinformationsverwaltung
(Windows) oder Ihrem Schlüsselbund-Manager (Linux), wenn Sie sie ebenfalls loswerden wollen.

## Aus dem Quellcode bauen

Wenn Sie lieber selbst bauen: Die README des Repositorys beschreibt die Rust- und
Node-Toolchain, die Tauri-Voraussetzungen und den `make dev`-Ablauf. Beachten Sie, dass Builds
aus dem Quellcode Ihre **eigenen** Gmail-/Microsoft-OAuth-Zugangsdaten in `.env.local`
benötigen; die veröffentlichten Binärdateien sind bereits konfiguriert.

Zwei Build-Hinweise zur KI-Laufzeit:

- Windows- und Linux-Releases werden mit `DYNAMIC_BACKENDS=1` und `CARGO_FEATURES=vulkan`
  gebaut — das ergibt ein einziges Artefakt, das zur Laufzeit eine GPU nutzt. Für den Bau des
  Vulkan-Backends wird das Vulkan SDK benötigt — nur zur Bauzeit; Anwender installieren es
  nie.
- Es gibt außerdem ein Cargo-Feature `cuda` für alle, die von Hand einen NVIDIA-spezifischen
  Build kompilieren wollen. Die Release-Pipeline verwendet es nicht — die Binärdateien, die
  Sie herunterladen, sind die Vulkan-Variante.
