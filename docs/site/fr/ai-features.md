---
title: "Fonctions d'IA"
description: "Discutez avec votre boîte, générez des réponses, classez le courrier, extrayez des tâches — le tout sur un modèle que vous contrôlez."
weight: 40
---

Toutes les fonctions d'IA ci-dessous passent par le backend que vous avez choisi, et chacune
peut être désactivée séparément. Avec le backend intégré par défaut, aucun prompt ni aucun
e-mail ne quitte jamais votre machine.

## Choisir un backend {#choosing-a-backend}

**Paramètres → IA : backend et modèles** détermine où se déroule l'inférence :

- **Dans l'app (local)** — un runtime llama.cpp intégré. Rien à installer, pas de démon, pas
  de trafic réseau. C'est le choix par défaut. Il utilise automatiquement votre GPU quand il y
  en a un — Metal sur Apple Silicon, Vulkan sous Windows et Linux — et le CPU sinon.
- **Ollama (local)** — un serveur Ollama que vous faites déjà tourner sur
  `http://localhost:11434`. Utile si vous entretenez une bibliothèque de modèles partagée, ou
  sur les Mac Intel où le runtime intégré est absent.
- **OpenRouter (distant)** — une API cloud payante. Nécessite une clé d'API, gère un plafond
  budgétaire mensuel et envoie le contenu de vos e-mails à un tiers — elle reste donc
  désactivée tant que vous ne l'activez pas.

### Le catalogue de modèles {#the-model-catalog}

Le backend intégré télécharge les modèles depuis un catalogue sélectionné, chacun épinglé à
une somme de contrôle vérifiée :

| Modèle | Taille de téléchargement | Mémoire nécessaire pour l'exécuter |
|---|---|---|
| Qwen 3.5 4B *(recommandé)* | ~3,0 Go | 8 Go |
| Qwen 3.5 4B Q8 | ~4,6 Go | 12 Go |
| Qwen 3.5 9B | ~5,7 Go | 16 Go |
| Gemma 4 12B Instruct | ~6,7 Go | 16 Go |
| Qwen 3.5 27B | ~17,6 Go | 24 Go |
| Qwen 3.6 35B A3B | ~22,4 Go | 32 Go |
| Nomic Embed Text v1.5 *(embeddings, inclus)* | ~84 Mo | 1 Go |

La colonne de droite correspond au pic de mémoire pendant la réponse — poids plus fenêtre de
contexte — toujours supérieur au téléchargement. **Dans quelle** mémoire il doit tenir dépend
de votre matériel :

- **Apple Silicon** — mémoire unifiée, partagée entre CPU et GPU, via Metal. Comparez le
  chiffre à la mémoire totale de votre Mac.
- **Un GPU sous Windows ou Linux** — la **VRAM** de la carte, pas votre RAM système, via
  Vulkan. Une carte de 8 Go fait tourner la ligne 8 Go et rien au-dessus, quelle que soit la
  RAM de la machine.
- **Sans GPU** — la RAM système, sur le CPU. Cela fonctionne ; c'est simplement plus lent.

Les modèles trop volumineux pour la mémoire de votre système sont grisés dans le sélecteur.
Les gros modèles répondent mieux et tournent plus lentement — commencez par le modèle
recommandé et ne montez que si le matériel a de la marge. Les exigences complètes sont dans
[Installation](../installation/#with-local-ai).

### Réglages de performance

- **Maintenir le modèle chargé** — combien de temps le modèle reste en mémoire entre deux
  tours
  (30 minutes par défaut). Des valeurs plus élevées évitent le rechargement lent ; `0` le
  libère immédiatement et rend la mémoire aux autres applications.
- **Fenêtre de contexte** — combien de jetons le modèle peut traiter par tour. Plus grande,
  elle contient davantage d'e-mails récupérés et coûte plus de mémoire — c'est le premier
  réglage à baisser quand un modèle tient tout juste.
- **Mode raisonnement** — chaîne de pensée sur les modèles compatibles. Plus lent, plus
  précis, et vous pouvez afficher ou masquer la trace.
- **Limiter le traitement IA aux courriels récents** — ignore embeddings et classification
  pour le courrier de plus de N jours.

## Discuter avec votre boîte

Posez vos questions en langage naturel — *« qu'a dit l'avocat à propos du contrat ? »*,
*« résume ce fil »*, *« qui me doit encore une réponse ? »* — et obtenez une réponse citant
les e-mails sources. Les réponses arrivent en flux au fur et à mesure de leur génération.

Sous le capot, le chat combine la récupération (recherche sémantique sur vos e-mails indexés)
et des appels d'outils (interrogations directes de la base). Le mode de routage est
configurable :

- **Toujours RAG en premier** — le mode par défaut ; récupérer le contexte, puis répondre.
- **Auto** — une heuristique choisit récupération ou outils selon la question.
- **Toujours les outils en premier** — passer directement aux requêtes structurées.

Les utilisateurs avancés peuvent modifier le prompt système et les prompts de récupération
(réécriture de requête, reclassement) dans
**Paramètres → IA : backend et modèles → Prompts du chat**.

## Brouillons d'IA

Un bouton **Brouillon IA** à côté de Répondre à tous rédige une réponse ancrée dans le fil que
vous consultez. Configurez une **persona** (une phrase sur l'identité de rédaction), un
**style d'écriture**, ainsi que le ton et la longueur par défaut — ou remplacez tout le modèle
de prompt. Les brouillons arrivent dans l'éditeur pour relecture avant tout envoi.

## Classification {#classification}

Chaque e-mail entrant est étiqueté sur trois axes — **priorité**, **intention** et **sujet** —
si bien que la boîte se trie pratiquement d'elle-même et que les filtres intelligents ont de
quoi filtrer.

La classification fonctionne en deux couches :

1. Les **règles** correspondent à des motifs d'expéditeur ou d'objet (`*@*.beehiiv.com`,
   `*facture*`) et attribuent des étiquettes instantanément, sans appel au modèle.
2. **Le modèle** traite tout ce que les règles ne couvrent pas, avec un prompt d'instructions
   que vous pouvez modifier.

Vous choisissez quelles catégories Gmail sont classées, vous pouvez tout reclasser après avoir
modifié le prompt, et rattraper le courrier non classé à la demande.

## Recherche sémantique

Les e-mails sont indexés localement pour que la recherche corresponde au sens et pas seulement
aux mots-clés — décrivez ce dont vous vous souvenez et EmailOps le retrouve. Cela alimente
aussi « trouver des messages similaires » et l'étape de récupération du chat. Choisissez les
catégories indexées et reconstruisez l'index de zéro après un changement de modèle
d'embeddings, dans **Paramètres → Recherche IA**.

## Traduction

Des boutons de traduction apparaissent sur les e-mails rédigés dans une autre langue et dans
la fenêtre de rédaction. Le prompt de traduction est modifiable comme les autres.

## Tâches

*Expérimental.* EmailOps parcourt le courrier à la recherche d'actions, d'engagements et
d'échéances, et les rassemble dans un panneau Tâches. Comme les vrais engagements se trouvent
généralement dans ce que **vous** avez écrit, un mode « apprendre uniquement des e-mails que
j'ai écrits » existe. Vous pouvez exclure des expéditeurs et des étiquettes (les newsletters
le sont par défaut), plafonner le nombre de tâches par e-mail, limiter la profondeur
d'extraction et traiter à la demande le courrier plus ancien.

## Mémoire

*Expérimental.* Les faits que l'assistant apprend sur vos contacts, domaines et projets sont
conservés comme contexte de long terme, pour que le chat ne reparte pas de zéro à chaque fois.
Les faits candidats sont notés et promus au-delà d'un seuil ; ceux qui obtiennent une note
faible expirent. Tout ce qui a été appris est consultable, et l'ensemble du sous-système
dispose d'un interrupteur général.

## Lentilles

*Expérimental.* Des vues typées sur votre boîte — des projections structurées, enregistrées et
extraites par l'IA (par exemple « toutes les factures avec montant et échéance ») que vous
créez et exécutez depuis la barre latérale.

## Tout désactiver

**Paramètres → IA : backend et modèles → Fonctions IA** est un interrupteur général.
Désactivez-le et EmailOps fonctionne comme un client e-mail classique : pas de chat, pas de
classification, pas d'embeddings, aucun modèle chargé. Vos données d'IA locales sont
conservées au cas où vous le réactiveriez.
