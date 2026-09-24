---
title: "Fonctions d'IA"
description: "Discutez avec votre boîte, générez des réponses, classez le courrier, extrayez des tâches — le tout sur un modèle que vous contrôlez."
weight: 40
nav:
  choosing-a-backend: settings/ai
  the-model-catalog: settings/ai
  performance-knobs: settings/ai
  chat-with-your-mailbox: view/chat
  ai-drafts: settings/aidrafts
  classification: settings/classification
  tag-board: view/tagboard
  semantic-search: settings/aisearch
  translation: settings/aitranslation
  tasks: settings/tasks
  memory: settings/memory
  lenses: settings/lenses
  turning-it-all-off: settings/ai
---

<!-- claim:ai-intro-1 -->
Toutes les fonctions d'IA ci-dessous passent par le backend que vous avez choisi, et chacune
peut être désactivée séparément. Avec le backend intégré par défaut, aucun prompt ni aucun
e-mail ne quitte jamais votre machine.

## Choisir un backend {#choosing-a-backend}

<!-- claim:ai-choosing-backend-1 -->
**Paramètres → IA : backend et modèles** détermine où se déroule l'inférence :

- **Dans l'app** — un runtime llama.cpp intégré. Rien à installer et pas de démon ; une fois
  le modèle téléchargé, répondre ne génère aucun trafic réseau. C'est le choix par défaut. Il utilise automatiquement votre GPU quand il y
  en a un — Metal sur Apple Silicon, Vulkan sous Windows et Linux — et le CPU sinon. Sur Mac,
  il exige une puce Apple Silicon (M1 ou plus récente) ; sur un Mac Intel il reste
  indisponible. <!-- claim:ai-choosing-backend-2 -->
- **Ollama** — un serveur Ollama que vous faites déjà tourner sur
  `http://localhost:11434`. Utile si vous entretenez une bibliothèque de modèles partagée. À
  noter : sur un Mac Intel, Ollama ne bénéficie pas non plus d'accélération GPU, il sera donc
  lent. <!-- claim:ai-choosing-backend-3 -->
- **OpenRouter** — une API cloud payante. Nécessite une clé d'API, gère un plafond
  budgétaire mensuel et envoie le contenu de vos e-mails à un tiers — elle reste donc
  désactivée tant que vous ne l'activez pas. Son panneau affiche les dépenses de la période en
  cours face à ce plafond, avec **Démarrer une nouvelle période** pour remettre le compteur à
  zéro. <!-- claim:ai-choosing-backend-4 -->

### Le catalogue de modèles {#the-model-catalog}

<!-- claim:ai-choosing-backend-model-catalog-1 -->
Le backend intégré télécharge les modèles depuis un catalogue sélectionné, chacun épinglé à
une somme de contrôle vérifiée :

<!-- generated:model-catalog -->
| Modèle | Taille de téléchargement | Mémoire exigée par EmailOps |
|---|---|---|
| Qwen 3.5 4B | ~3,0 Go | 8 Go |
| Qwen 3.5 4B Q8 | ~4,6 Go | 12 Go |
| Qwen 3.5 9B | ~5,7 Go | 16 Go |
| Gemma 4 12B Instruct | ~6,7 Go | 16 Go |
| Qwen 3.5 27B | ~17,6 Go | 24 Go |
| Qwen 3.6 35B A3B | ~22,4 Go | 32 Go |
| Nomic Embed Text v1.5 *(embeddings, inclus)* | ~84 Mo | 1 Go |
<!-- /generated:model-catalog -->

<!-- claim:ai-choosing-backend-model-catalog-2 -->
La colonne de droite est la mémoire qu'EmailOps exige avant de proposer un modèle — une marge
volontairement large, pas ce que le modèle consomme. Pic mesuré pendant la réponse, avec le
contexte attribué à un Mac de 16 Go : environ 3,7 Go pour Qwen 3.5 4B, 4,3 Go pour sa version
8 bits, 5,6 Go pour Qwen 3.5 9B et 7,1 Go pour Gemma 4 12B. **Dans quelle** mémoire il doit
tenir dépend de votre matériel :

- **Apple Silicon** — mémoire unifiée, partagée entre CPU et GPU, via Metal. Comparez le
  chiffre à la mémoire totale de votre Mac. <!-- claim:ai-choosing-backend-model-catalog-3 -->
- **Un GPU sous Windows ou Linux** — la **VRAM** de la carte, pas votre RAM système, via
  Vulkan. Une carte de 8 Go fait tourner la ligne 8 Go et rien au-dessus, quelle que soit la
  RAM de la machine. <!-- claim:ai-choosing-backend-model-catalog-4 -->
- **Sans GPU** — la RAM système, sur le CPU. Cela fonctionne ; c'est simplement plus lent. <!-- claim:ai-choosing-backend-model-catalog-5 -->

<!-- claim:ai-choosing-backend-model-catalog-6 -->
Un modèle porte la mention **Recommandé**, choisie pour la machine que vous utilisez :
EmailOps examine la mémoire système et, si vous avez une carte graphique dédiée, sa mémoire
également, puis propose le plus grand modèle qui tient confortablement. Un portable et une
station de travail verront donc des suggestions différentes. Les gros modèles répondent mieux
et tournent plus lentement : la mention est un point de départ, pas une règle. Les exigences
complètes sont dans [Installation](../installation/#with-local-ai).

### Réglages de performance {#performance-knobs}

- **Maintenir le modèle chargé** — combien de temps le modèle reste en mémoire entre deux
  tours
  (30 minutes par défaut). Des valeurs plus élevées évitent le rechargement lent ; `0` le
  libère immédiatement et rend la mémoire aux autres applications. <!-- claim:ai-choosing-backend-performance-knobs-1 -->
- **Fenêtre de contexte** — combien de jetons le modèle peut traiter par tour. Plus grande,
  elle contient davantage d'e-mails récupérés et coûte plus de mémoire — c'est le premier
  réglage à baisser quand un modèle tient tout juste. <!-- claim:ai-choosing-backend-performance-knobs-2 -->
- **Mode raisonnement** — chain-of-thought sur les modèles compatibles. Plus lent, plus
  précis, et vous pouvez afficher ou masquer la trace. <!-- claim:ai-choosing-backend-performance-knobs-3 -->
- **Limiter le traitement IA** — borne ce que couvrent embeddings et classification : tous
  les e-mails d'un compte jusqu'à une limite d'e-mails (1000 par défaut) et, pour les comptes
  plus volumineux, seulement le courrier plus récent qu'une limite en jours (365 par défaut). <!-- claim:ai-choosing-backend-performance-knobs-4 -->

## Discuter avec votre boîte {#chat-with-your-mailbox}

<!-- claim:ai-chat-mailbox-1 -->
Posez vos questions en langage naturel — *« qu'a dit l'avocat à propos du contrat ? »*,
*« résume ce fil »*, *« qui me doit encore une réponse ? »* — et obtenez une réponse citant
les e-mails sources. Les réponses arrivent en flux au fur et à mesure de leur génération.
**Afficher dans la liste des e-mails**, sous une réponse, place dans la liste exactement les
e-mails qu'elle cite, pour que vous puissiez les ouvrir et les traiter.

<!-- claim:ai-chat-mailbox-2 -->
Le chat occupe un panneau redimensionnable ancré à droite de la boîte de réception : vous
pouvez continuer à lire tout en posant vos questions, et une vue plein écran reste
disponible pour les sessions plus longues. Lorsqu'un e-mail est ouvert, le panneau propose
ce fil comme contexte via une puce que vous pouvez retirer : les questions sur cet e-mail
trouvent leur réponse dans le fil, et une question sur le reste de votre boîte
(*« qu'est-ce qui est arrivé aujourd'hui ? »*) continue d'y chercher. Ce contexte ne vaut
que pour une question et n'est jamais enregistré dans la conversation : vous pouvez donc
passer d'un e-mail à l'autre au sein d'un même chat.

<!-- claim:ai-chat-mailbox-3 -->
Le chat interroge un compte à la fois, et un sélecteur indique lequel — une réponse ne
provient donc jamais silencieusement de la mauvaise boîte. Chaque compte conserve sa propre
conversation tant que l'application reste ouverte : changer de compte vous ramène là où vous
en étiez, et non à un chat vide.

<!-- claim:ai-chat-mailbox-10 -->
Le chat répond aussi aux questions sur EmailOps lui-même — *« comment connecter Ollama ? »*,
*« où sont stockées mes données ? »*, *« que montre le tableau des étiquettes ? »* — à partir de ces
guides, dans votre langue et sans chercher dans votre boîte. La réponse renvoie à la section
du guide utilisée, et suivre le lien ouvre le paramètre ou la vue correspondants. Lorsqu'un
e-mail est ouvert comme contexte, le chat répond à partir de ce seul fil : retirez la puce
pour poser une question sur l'application. Désactivez-le avec
**Répondre aux questions sur EmailOps** dans **Paramètres → IA : backend et modèles** ; le
chat ne connaît alors que votre boîte.

<!-- claim:ai-chat-mailbox-11 -->
Chaque réponse dispose d'un panneau **Afficher le raisonnement** qui liste ce qui s'est
passé, dans l'ordre : la route suivie par la question et ce qui l'a décidée, le planificateur
de requêtes, la recherche dans la boîte, les sections des guides utilisées, chaque appel au
modèle avec sa durée et chaque appel d'outil avec ses arguments et son résultat.

<!-- claim:ai-chat-mailbox-12 -->
Le mode **Recherche** sert aux questions qui demandent tous les e-mails correspondants, et non
les quelques-uns qu'une réponse normale lit : *« liste toutes les factures de cette année »*,
*« combien de clients ont demandé un devis ? »*. Il estime d'abord combien d'e-mails il lirait
et combien de temps cela prendrait, et demande confirmation avant une exécution importante. Il
les lit ensuite par lots, et les listes et les décomptes sont exacts, avec un lien vers chaque
conversation. **Annuler la recherche** interrompt l'exécution.

<!-- claim:ai-chat-mailbox-13 -->
Demandez quelque chose pour lequel l'application a un formulaire — *« crée une Lens qui suit
les factures fournisseurs avec montant et date »* — et le vrai formulaire s'ouvre avec les
champs remplis, pour que vous le relisiez et l'enregistriez. Le chat ne crée jamais rien de
lui-même.

<!-- claim:ai-chat-mailbox-14 -->
**Arrêter la génération** interrompt une réponse en cours d'écriture ; ce qui était déjà
affiché est conservé.

<!-- claim:ai-chat-mailbox-4 -->
Sous le capot, le chat combine la récupération (recherche sémantique sur vos e-mails indexés)
et des appels d'outils (interrogations directes de la base). Le mode de routage est
configurable :

- **Toujours RAG en premier** — le mode par défaut ; récupérer le contexte, puis répondre. <!-- claim:ai-chat-mailbox-5 -->
- **Auto** — une heuristique décide, question par question, s'il faut d'abord récupérer du
  contexte. <!-- claim:ai-chat-mailbox-6 -->
- **Toujours les outils en premier** — passer directement aux requêtes structurées, sans
  récupération. <!-- claim:ai-chat-mailbox-7 -->

<!-- claim:ai-chat-mailbox-8 -->
Quel que soit le mode, les outils restent disponibles ; le mode décide seulement si la
récupération a lieu avant la réponse.

<!-- claim:ai-chat-mailbox-9 -->
Les utilisateurs avancés peuvent modifier le prompt système et les prompts de récupération
(réécriture de requête, reclassement) dans
**Paramètres → IA : backend et modèles → Prompts du chat**.

## Brouillons d'IA {#ai-drafts}

<!-- claim:ai-ai-drafts-1 -->
Un bouton **Brouillon IA** à côté de Répondre à tous rédige une réponse ancrée dans le fil que
vous consultez. Configurez une **persona** (une phrase sur l'identité de rédaction) et un
**style d'écriture** — ou remplacez tout le modèle de prompt. Les brouillons arrivent dans l'éditeur pour relecture avant tout envoi.

<!-- claim:ai-ai-drafts-2 -->
Un brouillon lit le fil jusqu'au message auquel vous répondez, jamais les réponses
postérieures. Vous pouvez dire à l'IA quoi répondre avant qu'elle écrive, et **Régénérer**
le réécrit.

## Classification {#classification}

<!-- claim:ai-classification-1 -->
Chaque e-mail entrant est étiqueté sur trois axes — **priorité**, **intention** et **sujet** —
si bien que la boîte se trie pratiquement d'elle-même et que les filtres intelligents ont de
quoi filtrer.

<!-- claim:ai-classification-2 -->
La classification fonctionne en deux couches :

1. Les **Règles** correspondent à des motifs d'expéditeur ou d'objet (`*@*.beehiiv.com`,
   `*facture*`) et attribuent des étiquettes instantanément, sans appel au modèle. <!-- claim:ai-classification-3 -->
2. **Le modèle** traite tout ce que les règles ne couvrent pas, avec un prompt d'instructions
   que vous pouvez modifier. <!-- claim:ai-classification-4 -->

<!-- claim:ai-classification-5 -->
Vous choisissez quelles catégories Gmail sont classées, vous pouvez tout reclasser après avoir
modifié le prompt, et rattraper le courrier non classé à la demande.

## Tableau d'étiquettes {#tag-board}

<!-- claim:tag-board-dimensions -->
Le **Tableau d'étiquettes** (sous **Vues** dans la barre latérale, à côté de la boîte de
réception) transforme ces étiquettes en tableau. Choisissez une dimension — **Entreprise**,
**Priorité**, **Intention** ou **Sujet** — et chaque valeur d'étiquette devient un bloc qui
liste ses fils ; dans **Tous les comptes**, vous obtenez un bloc par compte et par étiquette.
Un fil ne figure que dans un seul bloc, sous l'étiquette de son message classé le plus
récent.

<!-- claim:ai-tag-board-2 -->
Les blocs sont ordonnés selon l'attention réelle que reçoit une étiquette — à quelle
fréquence vous répondez à ses fils et les lisez, avec plus de poids pour l'activité récente —
les promotions et notifications venant en dernier. Les filtres intelligents de la barre
latérale suivent le même ordre. Glissez les blocs pour les réordonner (l'ordre est mémorisé
par dimension), masquez une étiquette depuis son menu ⋮ — l'étiquette suivante monte prendre
sa place, et le filtre quitte aussi la barre latérale — et récupérez les étiquettes masquées
avec le lien **Afficher les étiquettes masquées**.

<!-- claim:tag-board-toolbar -->
La barre d'outils restreint le tableau par période (**Aujourd'hui**, **Hier**, **7 derniers
jours** ou une plage de dates personnalisée), par catégorie Gmail, par nom d'étiquette, et
avec le même interrupteur **Masquer les indésirables** que la boîte de réception ; deux
icônes règlent la largeur des blocs. Un clic sur une carte ouvre le fil dans le volet de
lecture, son menu ⋮ propose les mêmes actions qu'une ligne de la boîte de réception, et
l'icône de chat du volet de lecture démarre une conversation avec ce fil en contexte.

<!-- claim:ai-tag-board-4 -->
Le tableau a besoin de la classification : il reste vide tant que le courrier n'est pas
étiqueté, et n'apparaît pas lorsque les fonctions d'IA sont désactivées.

## Recherche sémantique {#semantic-search}

<!-- claim:ai-semantic-search-1 -->
Les e-mails sont indexés localement pour que la recherche corresponde au sens et pas seulement
aux mots-clés — décrivez ce dont vous vous souvenez et EmailOps le retrouve. Cela alimente aussi l'étape de récupération du chat. Choisissez les
catégories indexées et reconstruisez l'index de zéro après un changement de modèle
d'embeddings, dans **Paramètres → Recherche IA**.

## Traduction {#translation}

<!-- claim:ai-translation-1 -->
Des boutons de traduction apparaissent sur les e-mails rédigés dans une autre langue et dans
la fenêtre de rédaction. Le prompt de traduction est modifiable comme les autres.

## Tâches {#tasks}

<!-- claim:ai-tasks-1 -->
*Expérimental.* EmailOps parcourt le courrier à la recherche d'actions, d'engagements et
d'échéances, et les rassemble dans un panneau Tâches. Comme les vrais engagements se trouvent
généralement dans ce que **vous** avez écrit, un mode « apprendre uniquement des e-mails que
j'ai écrits » existe. Vous pouvez exclure des expéditeurs et des étiquettes (les newsletters
le sont par défaut), plafonner le nombre de tâches par e-mail, limiter la profondeur
d'extraction et traiter à la demande le courrier plus ancien.

<!-- claim:ai-tasks-2 -->
Activez-les dans **Paramètres → Tâches IA** avec **Extraire des tâches des courriels** ; les
tâches trouvées apparaissent sous **Tâches** dans la barre latérale.

## Mémoire {#memory}

<!-- claim:ai-memory-1 -->
*Expérimental.* Les faits que l'assistant apprend sur vos contacts, domaines et projets sont
conservés comme contexte de long terme, pour que le chat ne reparte pas de zéro à chaque fois.
Les faits candidats sont notés et promus au-delà d'un seuil ; ceux qui obtiennent une note
faible expirent. Tout ce qui a été appris est consultable, et l'ensemble du sous-système
dispose d'un interrupteur général.

<!-- claim:ai-memory-2 -->
Activez-la dans **Paramètres → Mémoire de l'IA** avec **Laisser l'assistant retenir des
informations** ; ce qu'elle a appris est listé sous **Mémoire** dans la barre latérale.

## Lentilles {#lenses}

<!-- claim:ai-lenses-1 -->
*Expérimental.* Des vues typées sur votre boîte — des projections structurées, enregistrées et
extraites par l'IA (par exemple « toutes les factures avec montant et échéance ») que vous
créez et exécutez depuis la barre latérale. Une ligne exclue disparaît de la vue ; **Afficher
les lignes exclues** les réaffiche pour que vous puissiez en réintégrer une.

<!-- claim:ai-lenses-2 -->
Activez-les dans **Paramètres → Filtres dynamiques IA**, puis créez et exécutez chaque vue
depuis l'entrée **Filtres dynamiques** de la barre latérale.

<!-- claim:ai-lenses-3 -->
Une Lens peut être limitée à certains **Dossiers** d'un compte, dossiers IMAP personnalisés
compris.

## Tout désactiver {#turning-it-all-off}

<!-- claim:ai-turning-off-1 -->
**Paramètres → IA : backend et modèles → Fonctions IA** est un interrupteur général.
Désactivez-le et EmailOps fonctionne comme un client e-mail classique : pas de chat, pas de
classification, pas d'embeddings, aucun modèle chargé. Vos données d'IA locales sont
conservées au cas où vous le réactiveriez.
