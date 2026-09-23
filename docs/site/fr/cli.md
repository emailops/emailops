---
title: 'Ligne de commande (emailops-cli)'
description: "Scriptez et automatisez votre boîte depuis le terminal, avec une sortie JSON stable pour les scripts et les agents."
weight: 50
---

<!-- claim:cli-intro-1 -->
`emailops-cli` pilote le même moteur local que l'application de bureau — votre courrier, vos
comptes, votre IA locale — depuis un terminal. Il lit la base de données que l'application a
déjà synchronisée : pas de configuration séparée, pas de seconde copie de votre courrier.

<!-- claim:cli-intro-2 -->
macOS uniquement pour l'instant.

## Installation

<!-- claim:cli-install-1 -->
Téléchargez `EmailOps-CLI-macos.dmg` depuis la
[dernière version](https://github.com/emailops/emailops/releases/latest), montez-le et placez
le binaire dans votre `PATH` :

```bash
hdiutil attach ~/Downloads/EmailOps-CLI-macos.dmg
cp /Volumes/EmailOps\ CLI/emailops-cli /usr/local/bin/emailops-cli
hdiutil detach /Volumes/EmailOps\ CLI

emailops-cli doctor    # vérifie qu'il voit vos données et vos comptes
```

<!-- claim:cli-install-2 -->
Le binaire est universel (Apple Silicon + Intel), signé et notarié : Gatekeeper le laisse
passer sans invite.

## Démarrage rapide

<!-- claim:cli-quick-start-1 -->
```bash
emailops-cli accounts                     # quels comptes sont connectés
emailops-cli emails --limit 10            # les 10 e-mails les plus récents
emailops-cli search "facture"             # recherche plein texte
emailops-cli chat "qu'a dit Acme à propos du contrat ?"
emailops-cli                              # sans sous-commande → REPL interactif
```

<!-- claim:cli-quick-start-2 -->
Dans le REPL, toute action est une commande préfixée par `/` : `/chat <question>` est un tour
de chat (les jetons arrivent en direct) et les autres correspondent aux sous-commandes — `/search`, `/account`, `/sync`,
`/help`, `/quit`.

## Commandes

<!-- claim:cli-commands-1 -->
| Commande | Rôle |
|---|---|
| `accounts` | Liste les comptes configurés |
| `emails [--limit N] [--mailbox inbox\|sent\|spam\|trash]` | Liste les e-mails récents |
| `show <id>` | Affiche un e-mail (en-têtes et corps) |
| `search <requête> [--limit N]` | Recherche plein texte |
| `chat <question> [--trace]` | Pose une question ; `--trace` ajoute les temps de routage et de récupération |
| `sync [compte]` | Télécharge le courrier nouveau |
| `calendar [--days N] [--next] [--sync]` | Événements à venir (`--next` = prochaine réunion seulement) |
| `classify [--all]` | Classe les e-mails nouveaux — ou tous |
| `embed [--batch N]` | Génère les embeddings de recherche |
| `doctor` | Rapport d'état en lecture seule (base, comptes, configuration IA) |

<!-- claim:cli-commands-2 -->
Les options globales fonctionnent avant ou après la sous-commande : `--json`, `--quiet`,
`--account <id|email>`, `--model <modèle>`, `--data-dir <dossier>`.

<!-- claim:cli-commands-3 -->
Les commandes de lecture sont sûres pendant que l'application est ouverte. Les écritures
lourdes (`sync`, `classify`, `embed`) valent mieux application fermée.

## Scripter avec `--json`

<!-- claim:cli-scripting-json-1 -->
Avec `--json`, chaque commande écrit exactement une enveloppe sur stdout — même forme en cas
de succès ou d'échec — tandis que les journaux partent sur stderr :

```jsonc
{ "ok": true,  "data": { /* résultat */ }, "error": null }
{ "ok": false, "data": null, "error": { "code": "not_found", "message": "…", "params": {} } }
```

<!-- claim:cli-scripting-json-2 -->
```bash
# Objets des 20 e-mails les plus récents
emailops-cli emails --limit 20 --json | jq -r '.data[].subject'

# Uniquement le texte de la réponse à une question
emailops-cli chat "résume mes e-mails non lus" --json | jq -r '.data.answer'

# Expéditeur et objet de chaque résultat de recherche, en TSV
emailops-cli search "from:ana facture" --json | jq -r '.data[] | [.sender, .subject] | @tsv'
```

<!-- claim:cli-scripting-json-3 -->
Les codes de sortie sont regroupés selon ce qu'il y a à faire : `0` succès, `2` entrée
invalide, `3` introuvable, `4` authentification, `5` réseau/synchronisation, `6` IA, `130`
annulé, `1` tout le reste — les scripts peuvent donc se baser sur le code plutôt que d'analyser
du texte.

<!-- claim:cli-scripting-json-4 -->
Si vous avez plusieurs comptes, enregistrez-en un par défaut au lieu de répéter `--account` :

```bash
emailops-cli config set default-account vous@exemple.com
```
