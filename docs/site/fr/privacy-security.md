---
title: 'Confidentialité et sécurité'
description: "Où votre courrier est stocké, ce qui quitte votre machine, et les protections contre le courrier lui-même."
weight: 45
nav:
  where-your-data-is-stored: settings/privacy
  protection-from-the-mail-itself: settings/privacy
  locking-the-app: settings/privacy
---

<!-- claim:priv-intro-1 -->
EmailOps repose sur une règle : votre courrier reste sur votre machine. Cette page décrit ce
que cela signifie concrètement — où les données sont écrites, quels appels réseau existent et
quelles protections vous pouvez activer.

## Où vos données sont stockées {#where-your-data-is-stored}

<!-- claim:priv-where-data-1 -->
Tout réside dans le répertoire de données applicatives de votre système :

| Plateforme | Emplacement |
|---|---|
| macOS | `~/Library/Application Support/com.emailops.app` |
| Windows | `%APPDATA%\com.emailops.app` |
| Linux | `~/.local/share/com.emailops.app` |

<!-- claim:priv-where-data-2 -->
À l'intérieur :

- **Une base SQLite** — messages, fils, contacts, événements d'agenda, étiquettes de
  classification, embeddings de recherche et mémoire de l'IA. C'est la seule copie que
  conserve EmailOps. <!-- claim:priv-where-data-3 -->
- **Un dossier `models/`** — les modèles d'IA que vous avez téléchargés. <!-- claim:priv-where-data-4 -->

<!-- claim:priv-where-data-5 -->
Pointez `EMAILOPS_DATA_DIR` ailleurs avant le lancement pour utiliser un autre emplacement —
un second profil, ou un volume chiffré.

<!-- claim:priv-where-data-6 -->
**Les identifiants ne s'y trouvent pas.** Les jetons OAuth et les mots de passe IMAP vont dans
le magasin d'identifiants du système : Trousseau macOS, Gestionnaire d'identifiants Windows,
ou un trousseau Secret Service sous Linux. Ils ne sont jamais écrits dans un fichier de
configuration et survivent à la désinstallation de l'application.

## Il n'y a pas de serveur EmailOps

<!-- claim:priv-there-no-1 -->
Il n'y a pas de compte à créer, pas d'inscription et aucun backend exploité par nous — donc
nulle part où votre courrier puisse être téléversé, et rien à compromettre. L'application
communique exactement avec ces destinations, toutes identifiables :

| Destination | Quand | Contient votre courrier ? |
|---|---|---|
| Votre fournisseur de messagerie (Gmail, Microsoft Graph, votre serveur IMAP/SMTP) | À chaque synchronisation et envoi | Oui — c'est votre boîte |
| Votre fournisseur d'agenda (Google, Outlook) | Synchronisation du calendrier, si activée | Données d'agenda uniquement |
| Hugging Face | Uniquement pendant le téléchargement d'un modèle que vous avez choisi | Non |
| OpenRouter | Uniquement si vous basculez le fournisseur d'IA dessus | **Oui — les prompts contiennent le contenu des e-mails** |

<!-- claim:priv-there-no-2 -->
La dernière ligne est le seul chemin par lequel votre courrier peut atteindre un tiers ; il est
désactivé par défaut et exige une modification délibérée dans
**Paramètres → IA : backend et modèles** ainsi que votre propre clé d'API.

## Ce qu'EmailOps modifie dans votre boîte

<!-- claim:priv-what-emailops-1 -->
L'essentiel de ce que fait EmailOps est en lecture seule : l'application télécharge votre
courrier et en conserve une copie locale. Quelques actions atteignent délibérément le compte, pour
que ce que vous faites ici soit ce que vous voyez partout ailleurs :

| Action | Effet sur le compte |
|---|---|
| Marquer un message comme lu ou non lu | Le même message est marqué comme lu sur le compte (Gmail) |
| Supprimer un message | Le message est placé dans la **Corbeille** du compte (Gmail), où il reste récupérable pendant 30 jours |
| Déplacer un message vers un autre dossier, ou **Confirmer** | Le message est aussi déplacé sur le compte — Confirmer le range dans le dossier des indésirables du fournisseur |
| Créer, renommer ou supprimer un dossier | Le dossier change aussi sur le compte |
| Enregistrer un brouillon | Le brouillon est enregistré dans les Brouillons du compte |

<!-- claim:priv-what-emailops-2 -->
EmailOps ne supprime jamais définitivement un message : les suppressions vont toujours à la
corbeille, jamais à un effacement définitif. La lecture est appliquée d'abord en local pour que
l'application fonctionne hors ligne, et le compte se met à jour en arrière-plan. Tout le
reste — libellés, filtres, dossiers auxquels vous n'avez pas touché — reste exactement en
l'état.

<!-- claim:priv-what-emailops-3 -->
Chaque message envoyé depuis EmailOps se termine par une courte ligne « Envoyé avec
EmailOps » qui renvoie vers getemailops.com. Le lien porte `utm_source=email_footer`, qui
indique seulement à l'outil d'analyse du site que la visite vient d'un pied de message ; il
ne contient rien qui vous identifie, ni vous ni le destinataire.

## Aucune télémétrie

<!-- claim:priv-no-telemetry-1 -->
L'application ne collecte aucune donnée d'usage, n'envoie aucun rapport de plantage et ne
comporte aucun mécanisme de remontée dans les versions publiées. Il n'y a pas d'option de
refus parce qu'il n'y a rien à refuser. (Le dépôt contient une fonctionnalité optionnelle de
traçage OpenTelemetry pour le développement local ; elle est exclue de toutes les versions
publiées.)

## IA locale par défaut

<!-- claim:priv-local-ai-1 -->
Le backend par défaut exécute les modèles dans le processus même, via un runtime llama.cpp
intégré. Pas de démon, pas de serveur local, pas de socket réseau — le modèle lit vos e-mails
depuis le processus qui les détient déjà. Classification, brouillons, embeddings, chat,
extraction de tâches et de mémoire s'y exécutent tous.

<!-- claim:priv-local-ai-2 -->
Passer à Ollama garde également l'inférence en local, simplement dans un processus séparé sur
votre machine. Seul OpenRouter envoie du contenu hors de l'appareil. Voir
[choisir un backend](../ai-features/#choosing-a-backend).

## Protection contre le courrier lui-même {#protection-from-the-mail-itself}

<!-- claim:priv-protection-from-1 -->
L'e-mail est une surface d'attaque. Les défenses côté client :

- **Blocage du contenu distant** — images externes, pixels de suivi et autres ressources
  distantes sont bloqués jusqu'à autorisation. Une bannière par e-mail permet de les charger
  une fois, ou vous pouvez faire confiance à un expéditeur de façon permanente. C'est ce qui
  empêche un expéditeur de savoir quand et combien de fois vous avez ouvert un message.
  **Paramètres → Confidentialité et sécurité**
  liste, par compte, les expéditeurs à qui vous avez fait
  confiance, avec **Révoquer** qui rebloque leurs images. <!-- claim:priv-protection-from-2 -->
- **Notation des indésirables et du courrier de masse** — chaque message est noté localement
  pour le spam et le courrier de masse non désiré. Vos corrections
  (« indésirable » / « légitime ») l'entraînent. Le courrier signalé est estompé ou masqué,
  jamais supprimé ni déplacé sur le serveur sans confirmation explicite. <!-- claim:priv-protection-from-3 -->
- **Avertissements d'usurpation** — une vérification optionnelle qui signale les messages
  paraissant venir de quelqu'un d'autre. Désactivée par défaut, car c'est la seule
  vérification qui accuse un expéditeur de fraude et celle qui dispose du moins d'indices. <!-- claim:priv-protection-from-4 -->
- **Rendu assaini** — le HTML des messages est débarrassé des scripts, gestionnaires
  d'événements et objets embarqués avant affichage, des deux côtés de l'application. Les
  pièces jointes ne sont jamais ouvertes à votre place. <!-- claim:priv-protection-from-5 -->

## Verrouiller l'application {#locking-the-app}

<!-- claim:priv-locking-app-1 -->
Définissez un **mot de passe principal** dans **Paramètres → Confidentialité et sécurité** et
EmailOps reste verrouillé au démarrage jusqu'à sa saisie. Il n'existe aucune récupération — si
vous l'oubliez, vous réinstallez sur un répertoire de données neuf et resynchronisez depuis
votre fournisseur.

<!-- claim:priv-locking-app-2 -->
Soyons clairs sur ce que cela fait : cela verrouille l'application, cela ne chiffre **pas** la
base de données. Quiconque a accès à votre session déverrouillée et au répertoire de données
peut lire le fichier SQLite directement. Si cela fait partie de votre modèle de menace,
utilisez le chiffrement intégral du disque — FileVault sur macOS, BitLocker sur Windows, LUKS
sur Linux — c'est l'outil approprié.

## Vérifier tout cela

<!-- claim:priv-auditing-any-1 -->
EmailOps est sous Apache-2.0 et développé au grand jour. Les affirmations de cette page sont
vérifiables dans le code source sur
[github.com/emailops/emailops](https://github.com/emailops/emailops), tout comme le
comportement réseau — lancez-le derrière un proxy ou avec `tcpdump` et comparez au tableau
ci-dessus. Si quelque chose ne correspond pas,
[ouvrez un ticket](https://github.com/emailops/emailops/issues).
