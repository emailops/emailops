---
title: 'Installation'
description: "Téléchargez et installez EmailOps sur macOS, Windows ou Linux."
weight: 10
---

## Configuration requise

<!-- claim:inst-system-requirements-1 -->
Exécuter l'IA **en local** est la partie qui nécessite un matériel plus puissant, et c'est
optionnel. Vous pouvez la désactiver dans l'assistant de premier lancement et utiliser
EmailOps comme un client e-mail classique, ou conserver toutes les fonctions d'IA et les
diriger vers un fournisseur distant. Les deux modes ont des exigences très différentes.

### Avec IA locale {#with-local-ai}

<!-- claim:inst-system-requirements-local-ai-1 -->
L'une des exigences les plus importantes pour exécuter l'IA en local est la mémoire
disponible pour charger le modèle et son contexte. Selon votre machine, il s'agit d'un type
de mémoire ou d'un autre :

| | Mac Apple Silicon | Windows / Linux |
|---|---|---|
| Exécute le modèle sur | Le GPU intégré, via Metal | Votre GPU, via Vulkan — ou le CPU s'il n'y a pas de GPU |
| Mémoire dans laquelle il doit tenir | Mémoire unifiée, partagée avec le système | La **VRAM** du GPU, ou la RAM système sur CPU |
| Minimum | 8 Go unifiés | 8 Go de VRAM, ou 16 Go de RAM sans GPU |
| Recommandé | 16 Go unifiés ou plus | 12–16 Go de VRAM |
| Espace disque | ~3 Go — application et modèle de chat par défaut | ~3 Go — application et modèle de chat par défaut |

<!-- claim:inst-system-requirements-local-ai-2 -->
**Règle de dimensionnement :** le modèle doit tenir, en entier, dans la mémoire où il
s'exécute. Le **Qwen 3.5 4B** par défaut demande environ 8 Go pour que l'app le propose (il en utilise
moins de 4 pendant qu'il répond) ; le plus gros modèle du catalogue en réclame 32. Le chiffre de chaque modèle figure dans le
[catalogue de modèles](../ai-features/#the-model-catalog).

- **Apple Silicon** dispose d'une mémoire unifiée — le GPU adresse le même pool que le CPU, le
  chiffre à comparer est donc la mémoire totale du système. Un Mac de 16 Go fait tourner
  confortablement les modèles jusqu'à la ligne 16 Go, moins ce que macOS et vos autres
  applications utilisent déjà. <!-- claim:inst-system-requirements-local-ai-3 -->
- **Un GPU sous Windows ou Linux** possède sa propre VRAM, et c'est ce chiffre qui compte —
  32 Go de RAM système n'aident pas si la carte n'a que 8 Go. Un modèle qui ne tient pas
  déborde sur le CPU, ce qui fonctionne mais est plusieurs fois plus lent. <!-- claim:inst-system-requirements-local-ai-4 -->
- **Sans GPU du tout**, c'est pris en charge et il n'y a pas d'autre téléchargement à choisir.
  L'application se rabat sur le CPU et la RAM système ; prévoyez le chiffre du modèle en RAM
  et attendez-vous à des réponses nettement plus lentes. <!-- claim:inst-system-requirements-local-ai-5 -->

<!-- claim:inst-system-requirements-local-ai-6 -->
Les Mac Intel font exception : le moteur d'IA intégré nécessite une puce Apple Silicon
(M1 ou plus récente) et ne peut pas y fonctionner — voir la [note ci-dessous](#direct-download).

### Sans IA locale

<!-- claim:inst-system-requirements-without-local-1 -->
| | Minimum | Recommandé |
|---|---|---|
| RAM | 2 Go | 4 Go |
| Espace disque | ~500 Mo, plus la place pour le courrier synchronisé | Selon la taille de votre boîte |
| Processeur | 64 bits, 2 cœurs | — |
| Graphismes | Aucun | Aucun |

<!-- claim:inst-system-requirements-without-local-2 -->
Ce sont les exigences dans deux cas : IA entièrement désactivée, et IA activée **mais dirigée
vers OpenRouter**. L'inférence distante se déroule sur le matériel de quelqu'un d'autre, un
vieux portable suffit donc — au prix d'une clé d'API, d'un coût à l'usage et du contenu de vos
e-mails qui quitte l'appareil. Voir
[choisir un backend](../ai-features/#choosing-a-backend).

### Système d'exploitation

<!-- claim:inst-system-requirements-operating-system-1 -->
Les deux modes nécessitent l'un de ceux-ci :

- **macOS** Monterey (12) ou plus récent — Apple Silicon ou Intel. <!-- claim:inst-system-requirements-operating-system-2 -->
- **Windows** 10 ou 11, 64 bits. <!-- claim:inst-system-requirements-operating-system-3 -->
- **Linux** 64 bits, avec WebKitGTK et un trousseau Secret Service — voir
  [Linux](#linux) ci-dessous. <!-- claim:inst-system-requirements-operating-system-4 -->

## macOS

### Homebrew

<!-- claim:inst-macos-homebrew-1 -->
```bash
brew install --cask emailops/tap/emailops
```

<!-- claim:inst-macos-homebrew-2 -->
Mettez à jour ensuite avec `brew upgrade --cask emailops`.

### Téléchargement direct {#direct-download}

1. Téléchargez **EmailOps-macos.dmg** depuis la
   [dernière version](https://github.com/emailops/emailops/releases/latest). <!-- claim:inst-macos-direct-download-1 -->
2. Ouvrez le DMG et glissez **EmailOps.app** dans votre dossier Applications. <!-- claim:inst-macos-direct-download-2 -->
3. Lancez-le depuis Applications. <!-- claim:inst-macos-direct-download-3 -->

<!-- claim:inst-macos-direct-download-4 -->
> **Mac Intel :** ce téléchargement unique fonctionne sur tous les Mac — il n'y a pas de
> version Intel distincte. Les fonctions d'IA font exception : l'IA intégrée nécessite une puce
> Apple Silicon (M1 ou plus récente). Sur un Mac Intel elle reste désactivée et EmailOps vous
> explique pourquoi. Tout le reste fonctionne normalement. Pour l'IA, pointez EmailOps vers
> [OpenRouter](../ai-features/#choosing-a-backend).

## Windows

1. Téléchargez **EmailOps-windows-setup.exe** depuis la
   [dernière version](https://github.com/emailops/emailops/releases/latest). <!-- claim:inst-windows-1 -->
2. Lancez l'installeur et suivez les instructions. <!-- claim:inst-windows-2 -->
3. Ouvrez EmailOps depuis le menu Démarrer. <!-- claim:inst-windows-3 -->

### « Windows a protégé votre ordinateur » {#smartscreen}

<!-- claim:inst-windows-smartscreen-1 -->
Au lancement de l'installeur, Windows peut afficher un écran bleu **Microsoft Defender
SmartScreen** avec le message *« Windows a protégé votre ordinateur »*. C'est normal :
l'installeur n'est pas encore signé, car la signature sous Windows exige un certificat payant
dont le projet ne dispose pas. L'avertissement ne dit rien du fichier lui-même ; SmartScreen
l'affiche pour tout téléchargement non signé qu'il n'a pas encore vu souvent.

<!-- claim:inst-windows-smartscreen-2 -->
Pour continuer :

1. Cliquez sur **Informations complémentaires**. <!-- claim:inst-windows-smartscreen-3 -->
2. Vérifiez que l'application s'appelle **EmailOps**, puis cliquez sur **Exécuter quand même**. <!-- claim:inst-windows-smartscreen-4 -->

<!-- claim:inst-windows-smartscreen-5 -->
Si vous voulez d'abord vérifier que le téléchargement est authentique, comparez son empreinte
SHA-256 (`Get-FileHash .\EmailOps-windows-setup.exe` dans PowerShell) aux sommes de contrôle
de la [page de la version](https://github.com/emailops/emailops/releases/latest).

### Accélération GPU

<!-- claim:inst-windows-gpu-acceleration-1 -->
Il n'y a rien de plus à installer. La version Windows embarque un backend **Vulkan** qui se
charge à l'exécution dès qu'un pilote graphique fonctionnel est présent, et se rabat sur le
CPU sinon — un seul téléchargement dans les deux cas.

<!-- claim:inst-windows-gpu-acceleration-2 -->
Vulkan a été choisi plutôt que CUDA précisément pour que cela reste simple : il couvre AMD,
Intel et NVIDIA via le pilote graphique que vous avez déjà, sans kit constructeur à installer.
Gardez votre pilote GPU raisonnablement à jour et cela fonctionne.

## Linux {#linux}

1. Téléchargez **EmailOps-linux.AppImage** depuis la
   [dernière version](https://github.com/emailops/emailops/releases/latest). <!-- claim:inst-linux-1 -->
2. Rendez-le exécutable et lancez-le : <!-- claim:inst-linux-2 -->

```bash
chmod +x EmailOps-linux.AppImage
./EmailOps-linux.AppImage
```

### Accélération GPU

<!-- claim:inst-linux-gpu-acceleration-1 -->
Comme sous Windows : l'AppImage embarque un backend **Vulkan** utilisé automatiquement quand
un pilote graphique est présent, avec repli sur le CPU sinon. Pas de kit CUDA, pas de SDK
constructeur, pas de version distincte à choisir.

<!-- claim:inst-linux-gpu-acceleration-2 -->
Ce qu'il vous faut, c'est la pile de pilotes Vulkan ordinaire de votre carte —
`mesa-vulkan-drivers` sur AMD et Intel, le pilote propriétaire NVIDIA sur NVIDIA — que la
plupart des distributions de bureau installent déjà. Si `vulkaninfo` signale un périphérique,
EmailOps l'utilisera.

### Un trousseau est nécessaire

<!-- claim:inst-linux-keyring-required-1 -->
EmailOps n'écrit jamais les identifiants de compte dans un fichier : les jetons OAuth et les
mots de passe IMAP vont dans le magasin d'identifiants du système. macOS et Windows en
fournissent un (Trousseau et Gestionnaire d'identifiants) ; sous Linux, c'est à vous de le
fournir.

<!-- claim:inst-linux-keyring-required-2 -->
Il vous faut un fournisseur **Secret Service** installé et déverrouillé. N'importe lequel de
ceux-ci convient :

- **GNOME Keyring** (`gnome-keyring`) — la valeur par défaut sur GNOME, Ubuntu, Fedora
  Workstation. <!-- claim:inst-linux-keyring-required-3 -->
- **KWallet** (`kwalletmanager` avec l'interface Secret Service) — l'équivalent KDE. <!-- claim:inst-linux-keyring-required-4 -->
- **KeePassXC** avec *Paramètres → Intégration Secret Service* activée. <!-- claim:inst-linux-keyring-required-5 -->

<!-- claim:inst-linux-keyring-required-6 -->
Sur un gestionnaire de fenêtres minimal ou une session sans interface, il n'y a souvent aucun
trousseau en cours d'exécution. Installez l'un des précédents et assurez-vous qu'il est
déverrouillé au démarrage d'EmailOps — sinon l'ajout d'un compte échoue, faute d'endroit sûr
où placer les identifiants.

<!-- claim:inst-linux-keyring-required-7 -->
```bash
# Debian / Ubuntu
sudo apt install gnome-keyring

# Fedora
sudo dnf install gnome-keyring

# Arch
sudo pacman -S gnome-keyring
```

## Où vivent vos données

<!-- claim:inst-where-data-1 -->
Tout ce qu'EmailOps enregistre est sur votre machine, dans le répertoire de données
applicatives de votre système :

- **Courrier, contacts, événements d'agenda, embeddings** — une base SQLite locale. <!-- claim:inst-where-data-2 -->
- **Modèles d'IA téléchargés** — un dossier `models/` à côté de la base. <!-- claim:inst-where-data-3 -->
- **Jetons OAuth et mots de passe** — le trousseau de votre système, jamais un fichier en
  clair. <!-- claim:inst-where-data-4 -->

<!-- claim:inst-where-data-5 -->
Pour déplacer ou partager un répertoire de données (pour des tests, ou un second profil),
définissez la variable d'environnement `EMAILOPS_DATA_DIR` avant le lancement. Les chemins
exacts par plateforme, et ce qui est écrit où, figurent dans
[Confidentialité et sécurité](../privacy-security/#where-your-data-is-stored).

## Désinstaller

<!-- claim:inst-uninstalling-1 -->
Supprimer l'application laisse volontairement votre base de courrier et les modèles
téléchargés en place : une réinstallation reprend là où vous en étiez. Supprimez aussi le
répertoire de données pour repartir de zéro.

<!-- claim:inst-uninstalling-2 -->
Rien n'est supprimé chez votre fournisseur de messagerie dans un cas comme dans l'autre —
désinstaller EmailOps ne touche jamais au courrier sur Gmail, Outlook ou votre serveur IMAP.

### macOS

<!-- claim:inst-uninstalling-macos-1 -->
Avec Homebrew, une seule commande supprime l'application et ses données :

```bash
brew uninstall --zap --cask emailops
```

<!-- claim:inst-uninstalling-macos-2 -->
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

<!-- claim:inst-uninstalling-windows-1 -->
Désinstallez depuis **Paramètres → Applications → Applications installées → EmailOps**, ou
lancez le désinstalleur depuis l'entrée du menu Démarrer. Supprimez ensuite le répertoire de
données :

```
%APPDATA%\com.emailops.app
```

### Linux

<!-- claim:inst-uninstalling-linux-1 -->
Supprimez le fichier AppImage. Puis supprimez les répertoires de données et de configuration :

```bash
rm -rf ~/.local/share/com.emailops.app
rm -rf ~/.config/com.emailops.app
```

### Identifiants enregistrés

<!-- claim:inst-uninstalling-stored-credentials-1 -->
Sur toutes les plateformes, les jetons OAuth et les mots de passe IMAP résident dans le
trousseau du système plutôt que dans le répertoire de données : ils survivent donc à tout ce
qui précède. Supprimez les entrées `com.emailops.app` de Trousseaux d'accès (macOS), du
Gestionnaire d'identifiants (Windows) ou de votre gestionnaire de trousseau (Linux) si vous
voulez vous en débarrasser aussi.

## Compiler depuis les sources

<!-- claim:inst-building-from-1 -->
Si vous préférez le compiler vous-même, le README du dépôt couvre la chaîne d'outils Rust +
Node, les prérequis Tauri et le flux `make dev`. Notez que les compilations depuis les sources
nécessitent vos **propres** identifiants OAuth Gmail / Microsoft dans `.env.local` ; les
binaires publiés sont déjà configurés.

<!-- claim:inst-building-from-2 -->
Deux remarques de compilation sur le moteur d'IA :

- Les versions Windows et Linux sont compilées avec `DYNAMIC_BACKENDS=1` et
  `CARGO_FEATURES=vulkan`, ce qui produit un artefact unique capable d'exploiter un GPU à
  l'exécution. Compiler le backend Vulkan nécessite le SDK Vulkan — une dépendance de
  compilation uniquement ; les utilisateurs ne l'installent jamais. <!-- claim:inst-building-from-3 -->
- Une fonctionnalité Cargo `cuda` produit une variante réservée à NVIDIA. Le pipeline de
  publication la publie pour Windows sous forme d'installeur séparé,
  `EmailOps-windows-cuda.msi`, à côté de la version Vulkan par défaut, qui couvre tous les
  fabricants de GPU. <!-- claim:inst-building-from-4 -->
