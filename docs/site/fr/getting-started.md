---
title: 'Premiers pas'
description: "L'assistant de premier lancement : choisir un backend d'IA, télécharger un modèle et connecter votre première boîte."
weight: 20
---

Au premier lancement d'EmailOps, un assistant en quatre étapes se déclenche. Comptez quelques
minutes, dont l'essentiel est le téléchargement d'un modèle en arrière-plan.

## 1. IA activée ou non

EmailOps inspecte votre matériel et recommande d'activer ou non l'IA locale. Choisissez :

- **IA activée** — chat, brouillons, classification et recherche sémantique s'exécutent tous
  sur cette machine.
- **Client e-mail simple** — aucun modèle n'est téléchargé et aucun appel d'IA n'est jamais
  effectué. Vous pourrez activer l'IA plus tard dans **Réglages → Backend et modèles d'IA**,
  et la désactiver tout aussi facilement.

## 2. Backend et modèle d'IA

Si vous avez activé l'IA, choisissez où se déroule l'inférence :

| Backend | Ce que cela signifie |
|---|---|
| **Dans l'app (local)** | Par défaut. Un runtime llama.cpp intégré à EmailOps. Pas de démon, pas de configuration, pas de réseau. |
| **Ollama (local)** | Utilise votre serveur Ollama existant sur `http://localhost:11434`. |
| **OpenRouter (distant)** | Envoie les prompts à une API cloud payante. Optionnel, par fonction, désactivé par défaut. |

Avec le backend intégré, choisissez un modèle de chat dans le catalogue. **Qwen 3.5 4B** est
le choix recommandé par défaut : environ 3 Go à télécharger, il lui faut à peu près 8 Go de
mémoire pour tourner, et il prend en charge les appels d'outils dont dépend le chat. Les
modèles trop volumineux pour la mémoire de votre système sont grisés. Le téléchargement se
poursuit en arrière-plan — vous pouvez continuer l'assistant.

La mémoire qui compte dépend de la machine : **mémoire unifiée** sur un Mac Apple Silicon, la
**VRAM de votre GPU** sur une machine Windows ou Linux avec carte dédiée, et la RAM système
s'il n'y a pas de GPU. Le [catalogue de modèles](../ai-features/#the-model-catalog) indique le
chiffre pour chaque modèle.

Le modèle d'embeddings qui alimente la recherche sémantique (**Nomic Embed Text v1.5**,
~80 Mo) est livré dans l'application sur macOS : il n'y a rien à télécharger pour la
recherche.

## 3. Disposition de la boîte

Choisissez la disposition — **divisée** (liste à gauche, message à droite) ou **pleine
largeur** (un panneau à la fois). Modifiable à tout moment dans **Réglages → Apparence**, avec
la langue de l'interface (français, anglais, espagnol, allemand).

## 4. Connecter un compte

La dernière étape ajoute votre première boîte. EmailOps prend en charge :

- **Gmail** — connectez-vous dans votre navigateur et accordez l'accès. Les jetons vont
  directement dans le trousseau du système.
- **Outlook / Microsoft 365** — même parcours par navigateur, via l'API Microsoft Graph.
- **IMAP / SMTP** — iCloud, Yahoo, Fastmail, ProtonMail Bridge ou tout serveur personnalisé.
  Saisissez directement les paramètres du serveur et les identifiants.

Ajoutez d'autres comptes quand vous voulez depuis **Réglages → Comptes**. Avec plusieurs
comptes connectés, vous obtenez une boîte unifiée « Tous les comptes » en plus des vues par
compte.

## Après l'assistant

### La première synchronisation prend du temps

EmailOps télécharge votre courrier dans une base de données locale, et la première passe doit
tout récupérer depuis zéro. La durée dépend de la taille de la boîte — quelques minutes pour
un petit compte, nettement plus pour un compte avec des années d'historique et de grosses
pièces jointes. Cela tourne en arrière-plan et vous pouvez lire et rechercher ce qui est déjà
arrivé pendant que le reste se met à jour.

C'est un coût unique. Chaque synchronisation ultérieure est **incrémentale** : elle ne demande
à votre fournisseur que ce qui a changé depuis la dernière fois, donc elle se termine en
quelques secondes et tourne discrètement selon sa planification. Si l'IA est activée, la
classification et les embeddings rattrapent également le retard au premier lancement, puis ne
touchent plus que le courrier nouveau.

Une fois la première synchronisation terminée :

1. La **classification** commence à étiqueter le courrier nouveau par priorité, intention et
   sujet — voir [Fonctions d'IA](../ai-features/#classification).
2. Les **embeddings** sont générés en arrière-plan pour donner de la matière à la recherche
   sémantique. Vous pouvez suivre la progression et reconstruire l'index dans
   **Réglages → Recherche IA**.
3. Envisagez de définir un **mot de passe principal** dans
   **Réglages → Confidentialité et sécurité** si vous voulez que l'application se verrouille
   au démarrage — voir [Confidentialité et sécurité](../privacy-security/).

La classification comme les embeddings respectent une limite d'ancienneté
(**Réglages → Backend et modèles d'IA**) : une archive vieille de dix ans n'est pas traitée
sauf si vous le demandez.
