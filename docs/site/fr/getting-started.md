---
title: 'Premiers pas'
description: "L'assistant de premier lancement : choisir un backend d'IA, télécharger un modèle et connecter votre première boîte."
weight: 20
nav:
  ai-on-or-off: settings/ai
  ai-backend-and-model: settings/ai
  inbox-layout: settings/appearance
---

<!-- claim:start-intro-1 -->
Au premier lancement d'EmailOps, un assistant d'au plus quatre étapes se déclenche (trois si
vous choisissez un client de messagerie simple). Comptez quelques
minutes, dont l'essentiel est le téléchargement d'un modèle en arrière-plan.

## 1. IA activée ou non {#ai-on-or-off}

<!-- claim:start-1-ai-1 -->
EmailOps inspecte votre matériel et recommande d'activer ou non l'IA locale. Choisissez :

- **Utiliser l'IA** — chat, brouillons, classification et recherche sémantique s'exécutent tous
  sur cette machine. <!-- claim:start-1-ai-2 -->
- **Client de messagerie simple** — aucun modèle n'est téléchargé et aucun appel d'IA n'est jamais
  effectué. Vous pourrez activer l'IA plus tard dans **Paramètres → IA : backend et modèles**,
  et la désactiver tout aussi facilement. <!-- claim:start-1-ai-3 -->

## 2. Backend et modèle d'IA {#ai-backend-and-model}

<!-- claim:start-2-ai-1 -->
Si vous avez activé l'IA, choisissez où se déroule l'inférence :

| Backend | Ce que cela signifie |
|---|---|
| **Dans l'app** | Par défaut. Un runtime llama.cpp intégré à EmailOps. Pas de démon, pas de configuration, pas de réseau. |
| **Ollama** | Utilise votre serveur Ollama existant sur `http://localhost:11434`. |
| **OpenRouter** | Envoie les prompts à une API cloud payante. Optionnel, par fonction, désactivé par défaut. |

<!-- claim:start-2-ai-2 -->
Avec le backend intégré, choisissez un modèle de chat dans le catalogue. EmailOps présélectionne
le plus grand modèle que votre machine peut faire tourner confortablement : la recommandation
dépend donc de la mémoire détectée — sur une machine de 16 Go c'est **Qwen 3.5 4B**, environ
3 Go à télécharger et moins de 4 Go de mémoire pendant qu'il répond ; une machine plus généreuse se
voit proposer un modèle plus grand du même catalogue. Tous les modèles recommandés prennent en
charge les appels d'outils dont dépend le chat. Le téléchargement affiche sa progression sur place ; **Continuer** reste désactivé tant que le
modèle de chat n'est pas téléchargé, ou jusqu'à ce que vous choisissiez un fichier déjà présent
avec **Utiliser un fichier existant…**.

<!-- claim:start-2-ai-3 -->
La mémoire qui compte dépend de la machine : **mémoire unifiée** sur un Mac Apple Silicon, la
**VRAM de votre GPU** sur une machine Windows ou Linux avec carte dédiée, et la RAM système
s'il n'y a pas de GPU. Le [catalogue de modèles](../ai-features/#the-model-catalog) indique le
chiffre pour chaque modèle.

<!-- claim:start-2-ai-4 -->
Le modèle d'embeddings qui alimente la recherche sémantique (**Nomic Embed Text v1.5**,
~80 Mo) est livré dans l'application sur macOS : il n'y a rien à télécharger pour la
recherche.

## 3. Disposition de la boîte {#inbox-layout}

<!-- claim:start-3-inbox-1 -->
Choisissez la disposition — **divisée** (liste à gauche, message à droite) ou **pleine
largeur** (un panneau à la fois). Modifiable à tout moment dans **Paramètres → Apparence**, avec
la langue de l'interface (français, anglais, espagnol, allemand).

## 4. Connecter un compte

<!-- claim:start-4-connect-1 -->
La dernière étape ajoute votre première boîte. EmailOps prend en charge :

- **Gmail** — connectez-vous dans votre navigateur et accordez l'accès. Les jetons vont
  directement dans le trousseau du système. <!-- claim:start-4-connect-2 -->
- **Outlook / Microsoft 365** — même parcours par navigateur, via l'API Microsoft Graph. <!-- claim:start-4-connect-3 -->
- **IMAP / SMTP** — iCloud, Yahoo, Fastmail, ProtonMail Bridge ou tout serveur personnalisé.
  Saisissez directement les paramètres du serveur et les identifiants. <!-- claim:start-4-connect-4 -->

<!-- claim:start-4-connect-5 -->
Ajoutez d'autres comptes quand vous voulez avec le bouton **+** à côté de **Comptes** dans la
barre latérale. Avec plusieurs comptes connectés, vous obtenez une boîte unifiée « Tous les
comptes » en plus des vues par compte.

## Après l'assistant

### La première synchronisation prend du temps

<!-- claim:start-after-wizard-first-sync-1 -->
EmailOps télécharge votre courrier dans une base de données locale, et la première passe doit
tout récupérer depuis zéro. La durée dépend de la taille de la boîte — quelques minutes pour
un petit compte, nettement plus pour un compte avec des années d'historique et de grosses
pièces jointes. Cela tourne en arrière-plan et les premiers messages apparaissent en quelques secondes — le
courrier est téléchargé par tranches au fur et à mesure du parcours de la boîte, et non une
fois tout l'historique parcouru — vous pouvez donc lire et rechercher ce qui est déjà arrivé
pendant que le reste se met à jour.

<!-- claim:start-after-wizard-first-sync-2 -->
C'est un coût unique. Chaque synchronisation ultérieure est **incrémentale** : elle ne demande
à votre fournisseur que ce qui a changé depuis la dernière fois, donc elle se termine en
quelques secondes et tourne discrètement selon sa planification. Si l'IA est activée, la
classification et les embeddings rattrapent également le retard au premier lancement, puis ne
touchent plus que le courrier nouveau.

<!-- claim:start-after-wizard-first-sync-3 -->
Une fois la première synchronisation terminée :

1. La **classification** commence à étiqueter le courrier nouveau par priorité, intention et
   sujet — voir [Fonctions d'IA](../ai-features/#classification). <!-- claim:start-after-wizard-first-sync-4 -->
2. Les **embeddings** sont générés en arrière-plan pour donner de la matière à la recherche
   sémantique. Vous pouvez suivre la progression et reconstruire l'index dans
   **Paramètres → Recherche IA**. <!-- claim:start-after-wizard-first-sync-5 -->
3. Envisagez de définir un **mot de passe principal** dans
   **Paramètres → Confidentialité et sécurité** si vous voulez que l'application se verrouille
   au démarrage — voir [Confidentialité et sécurité](../privacy-security/). <!-- claim:start-after-wizard-first-sync-6 -->

<!-- claim:start-after-wizard-first-sync-7 -->
La classification comme les embeddings respectent **Limiter le traitement IA**
(**Paramètres → IA : backend et modèles**) : une archive vieille de dix ans n'est pas traitée
sauf si vous le demandez.
