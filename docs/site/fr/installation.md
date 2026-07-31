---
title: 'Installation'
description: "Téléchargez et installez EmailOps sur macOS, Windows ou Linux."
weight: 10
---

## Configuration requise

Exécuter l'IA **en local** est la partie exigeante, et c'est optionnel. Vous pouvez la
désactiver dans l'assistant de premier lancement et utiliser EmailOps comme un client e-mail
classique,
ou conserver toutes les fonctions d'IA et les diriger vers un fournisseur distant. Les deux
modes ont des exigences très différentes.

### Avec IA locale {#with-local-ai}

La mémoire qui compte est celle dans laquelle le modèle s'exécute, et elle diffère selon la
plateforme.

| | Mac Apple Silicon | Windows / Linux |
|---|---|---|
| Exécute le modèle sur | Le GPU intégré, via Metal | Votre GPU, via Vulkan — ou le CPU s'il n'y a pas de GPU |
| Mémoire dans laquelle il doit tenir | Mémoire unifiée, partagée avec le système | La **VRAM** du GPU, ou la RAM système sur CPU |
| Minimum | 8 Go unifiés | 8 Go de VRAM, ou 16 Go de RAM sans GPU |
| Recommandé | 16 Go unifiés ou plus | 12–16 Go de VRAM |
| Espace disque | ~3 Go — application et modèle de chat par défaut | ~3 Go — application et modèle de chat par défaut |

**Règle de dimensionnement :** le modèle doit tenir, en entier, dans la mémoire où il
s'exécute. Le **Qwen 3.5 4B** par défaut demande environ 8 Go ; le plus gros modèle du
catalogue en réclame 32. Le chiffre de chaque modèle figure dans le
[catalogue de modèles](../ai-features/#the-model-catalog).

- **Apple Silicon** dispose d'une mémoire unifiée — le GPU adresse le même pool que le CPU, le
  chiffre à comparer est donc la mémoire totale du système. Un Mac de 16 Go fait tourner
  confortablement les modèles jusqu'à la ligne 16 Go, moins ce que macOS et vos autres
  applications utilisent déjà.
- **Un GPU sous Windows ou Linux** possède sa propre VRAM, et c'est ce chiffre qui compte —
  32 Go de RAM système n'aident pas si la carte n'a que 8 Go. Un modèle qui ne tient pas
  déborde sur le CPU, ce qui fonctionne mais est plusieurs fois plus lent.
- **Sans GPU du tout**, c'est pris en charge et il n'y a pas d'autre téléchargement à choisir.
  L'application se rabat sur le CPU et la RAM système ; prévoyez le chiffre du modèle en RAM
  et attendez-vous à des réponses nettement plus lentes.

Les Mac Intel font exception : cette version est livrée sans le moteur d'IA intégré — voir la
[note ci-dessous](#direct-download).

### Sans IA locale

| | Minimum | Recommandé |
|---|---|---|
| RAM | 2 Go | 4 Go |
| Espace disque | ~500 Mo, plus la place pour le courrier synchronisé | Selon la taille de votre boîte |
| Processeur | 64 bits, 2 cœurs | — |
| Graphismes | Aucun | Aucun |

Ce sont les exigences dans deux cas : IA entièrement désactivée, et IA activée **mais dirigée
vers OpenRouter**. L'inférence distante se déroule sur le matériel de quelqu'un d'autre, un
vieux portable suffit donc — au prix d'une clé d'API, d'un coût à l'usage et du contenu de vos
e-mails qui quitte l'appareil. Voir
[choisir un backend](../ai-features/#choosing-a-backend).

### Système d'exploitation

Les deux modes nécessitent l'un de ceux-ci :

- **macOS** Monterey (12) ou plus récent — Apple Silicon ou Intel.
- **Windows** 10 ou 11, 64 bits.
- **Linux** 64 bits, avec WebKitGTK et un trousseau Secret Service — voir
  [Linux](#linux) ci-dessous.

## macOS

### Homebrew

```bash
brew install --cask emailops/tap/emailops
```

Mettez à jour ensuite avec `brew upgrade --cask emailops`.

### Téléchargement direct {#direct-download}

1. Téléchargez **EmailOps-macos.dmg** depuis la
   [dernière version](https://github.com/emailops/emailops/releases/latest).
2. Ouvrez le DMG et glissez **EmailOps.app** dans votre dossier Applications.
3. Lancez-le depuis Applications.

> **Mac Intel :** utilisez la version `EmailOps-macos-intel.dmg`. Elle n'inclut pas le moteur
> d'IA intégré ; pour des fonctions d'IA sur un Mac Intel, pointez EmailOps vers
> [Ollama ou OpenRouter](../ai-features/#choosing-a-backend).

## Windows

1. Téléchargez **EmailOps-windows.exe** depuis la
   [dernière version](https://github.com/emailops/emailops/releases/latest).
2. Lancez l'installeur et suivez les instructions.
3. Ouvrez EmailOps depuis le menu Démarrer.

### Accélération GPU

Il n'y a rien de plus à installer. La version Windows embarque un backend **Vulkan** qui se
charge à l'exécution dès qu'un pilote graphique fonctionnel est présent, et se rabat sur le
CPU sinon — un seul téléchargement dans les deux cas.

Vulkan a été choisi plutôt que CUDA précisément pour que cela reste simple : il couvre AMD,
Intel et NVIDIA via le pilote graphique que vous avez déjà, sans kit constructeur à installer.
Gardez votre pilote GPU raisonnablement à jour et cela fonctionne.

## Linux {#linux}

1. Téléchargez **EmailOps-linux.AppImage** depuis la
   [dernière version](https://github.com/emailops/emailops/releases/latest).
2. Rendez-le exécutable et lancez-le :

```bash
chmod +x EmailOps-linux.AppImage
./EmailOps-linux.AppImage
```

### Accélération GPU

Comme sous Windows : l'AppImage embarque un backend **Vulkan** utilisé automatiquement quand
un pilote graphique est présent, avec repli sur le CPU sinon. Pas de kit CUDA, pas de SDK
constructeur, pas de version distincte à choisir.

Ce qu'il vous faut, c'est la pile de pilotes Vulkan ordinaire de votre carte —
`mesa-vulkan-drivers` sur AMD et Intel, le pilote propriétaire NVIDIA sur NVIDIA — que la
plupart des distributions de bureau installent déjà. Si `vulkaninfo` signale un périphérique,
EmailOps l'utilisera.

### Un trousseau est nécessaire

EmailOps n'écrit jamais les identifiants de compte dans un fichier : les jetons OAuth et les
mots de passe IMAP vont dans le magasin d'identifiants du système. macOS et Windows en
fournissent un (Trousseau et Gestionnaire d'identifiants) ; sous Linux, c'est à vous de le
fournir.

Il vous faut un fournisseur **Secret Service** installé et déverrouillé. N'importe lequel de
ceux-ci convient :

- **GNOME Keyring** (`gnome-keyring`) — la valeur par défaut sur GNOME, Ubuntu, Fedora
  Workstation.
- **KWallet** (`kwalletmanager` avec l'interface Secret Service) — l'équivalent KDE.
- **KeePassXC** avec *Paramètres → Intégration Secret Service* activée.

Sur un gestionnaire de fenêtres minimal ou une session sans interface, il n'y a souvent aucun
trousseau en cours d'exécution. Installez l'un des précédents et assurez-vous qu'il est
déverrouillé au démarrage d'EmailOps — sinon l'ajout d'un compte échoue, faute d'endroit sûr
où placer les identifiants.

```bash
# Debian / Ubuntu
sudo apt install gnome-keyring

# Fedora
sudo dnf install gnome-keyring

# Arch
sudo pacman -S gnome-keyring
```

## Où vivent vos données

Tout ce qu'EmailOps enregistre est sur votre machine, dans le répertoire de données
applicatives de votre système :

- **Courrier, contacts, événements d'agenda, embeddings** — une base SQLite locale.
- **Modèles d'IA téléchargés** — un dossier `models/` à côté de la base.
- **Jetons OAuth et mots de passe** — le trousseau de votre système, jamais un fichier en
  clair.

Pour déplacer ou partager un répertoire de données (pour des tests, ou un second profil),
définissez la variable d'environnement `EMAILOPS_DATA_DIR` avant le lancement. Les chemins
exacts par plateforme, et ce qui est écrit où, figurent dans
[Confidentialité et sécurité](../privacy-security/#where-your-data-is-stored).

## Désinstaller

Supprimer l'application laisse volontairement votre base de courrier et les modèles
téléchargés en place : une réinstallation reprend là où vous en étiez. Supprimez aussi le
répertoire de données pour repartir de zéro.

Rien n'est supprimé chez votre fournisseur de messagerie dans un cas comme dans l'autre —
désinstaller EmailOps ne touche jamais au courrier sur Gmail, Outlook ou votre serveur IMAP.

### macOS

Avec Homebrew, une seule commande supprime l'application et ses données :

```bash
brew uninstall --zap --cask emailops
```

Sans `--zap`, seule l'application disparaît. À la main : glissez **EmailOps.app** depuis
Applications vers la Corbeille, puis supprimez :

```
~/Library/Application Support/com.emailops.app
~/Library/Caches/com.emailops.app
~/Library/HTTPStorages/com.emailops.app
~/Library/Preferences/com.emailops.app.plist
~/Library/Saved Application State/com.emailops.app.savedState
~/Library/WebKit/com.emailops.app
```

### Windows

Désinstallez depuis **Paramètres → Applications → Applications installées → EmailOps**, ou
lancez le désinstalleur depuis l'entrée du menu Démarrer. Supprimez ensuite le répertoire de
données :

```
%APPDATA%\com.emailops.app
```

### Linux

Supprimez le fichier AppImage. Puis supprimez les répertoires de données et de configuration :

```bash
rm -rf ~/.local/share/com.emailops.app
rm -rf ~/.config/com.emailops.app
```

### Identifiants enregistrés

Sur toutes les plateformes, les jetons OAuth et les mots de passe IMAP résident dans le
trousseau du système plutôt que dans le répertoire de données : ils survivent donc à tout ce
qui précède. Supprimez les entrées `com.emailops.app` de Trousseaux d'accès (macOS), du
Gestionnaire d'identifiants (Windows) ou de votre gestionnaire de trousseau (Linux) si vous
voulez vous en débarrasser aussi.

## Compiler depuis les sources

Si vous préférez le compiler vous-même, le README du dépôt couvre la chaîne d'outils Rust +
Node, les prérequis Tauri et le flux `make dev`. Notez que les compilations depuis les sources
nécessitent vos **propres** identifiants OAuth Gmail / Microsoft dans `.env.local` ; les
binaires publiés sont déjà configurés.

Deux remarques de compilation sur le moteur d'IA :

- Les versions Windows et Linux sont compilées avec `DYNAMIC_BACKENDS=1` et
  `CARGO_FEATURES=vulkan`, ce qui produit un artefact unique capable d'exploiter un GPU à
  l'exécution. Compiler le backend Vulkan nécessite le SDK Vulkan — une dépendance de
  compilation uniquement ; les utilisateurs ne l'installent jamais.
- Une fonctionnalité Cargo `cuda` existe également pour qui veut compiler à la main une
  version spécifique NVIDIA. Elle n'est pas utilisée par le pipeline de publication : les
  binaires que vous téléchargez sont ceux de Vulkan.
