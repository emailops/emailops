---
title: 'Dépannage'
description: "Solutions aux problèmes les plus fréquents : IA indisponible, chat lent, recherche par mots-clés uniquement, erreurs de synchronisation."
weight: 60
nav:
  ai-features-are-unavailable: settings/ai
  chat-is-slow: settings/ai
  gpu-not-used: settings/ai
  search-returns-keyword-results-only: settings/aisearch
  classification-is-not-tagging-anything: settings/classification
---

## Les fonctions d'IA sont indisponibles {#ai-features-are-unavailable}

<!-- claim:trbl-ai-features-1 -->
Avec le backend **intégré**, vérifiez que le modèle recommandé a fini de se télécharger dans
**Paramètres → IA : backend et modèles**. Un téléchargement interrompu n'est
jamais utilisé comme modèle : relancez-le depuis le même écran et il reprend là où il s'était
arrêté ; un téléchargement qui échoue à la vérification est supprimé automatiquement.

<!-- claim:trbl-ai-features-2 -->
Si vous êtes passé à **Ollama**, assurez-vous que le démon tourne et est joignable sur
`http://localhost:11434`, et que vous avez récupéré un modèle :

```bash
ollama pull llama3.2
ollama pull nomic-embed-text
```

<!-- claim:trbl-ai-features-3 -->
Sur un **Mac Intel**, l'IA intégrée ne peut pas fonctionner : elle nécessite une puce Apple
Silicon (M1 ou plus récente), donc EmailOps la laisse désactivée. Utilisez OpenRouter à la
place. Ollama s'installe, mais il ne bénéficie d'aucune accélération GPU sur Intel non plus :
attendez-vous à ce qu'il soit trop lent pour être agréable.

## Le chat est lent {#chat-is-slow}

<!-- claim:trbl-chat-slow-1 -->
L'inférence locale prend un temps réel — sur une machine modeste, une réponse peut demander
des dizaines de secondes. Ce qui aide, à peu près par ordre d'efficacité :

1. **Vérifiez que le modèle tient vraiment.** C'est le point principal. Sous Windows ou Linux,
   un modèle plus grand que la **VRAM** de votre GPU déborde sur le CPU et devient plusieurs
   fois plus lent — la solution est un modèle plus petit, pas plus de RAM système. Sur Apple
   Silicon, la comparaison se fait avec la mémoire unifiée totale. Voir le
   [catalogue de modèles](../ai-features/#the-model-catalog) pour le chiffre de chaque modèle. <!-- claim:trbl-chat-slow-2 -->
2. **Prenez un modèle plus petit.** Qwen 3.5 4B est le plus petit modèle de chat du
   catalogue. <!-- claim:trbl-chat-slow-3 -->
3. **Augmentez « Maintenir le modèle chargé »** dans les réglages d'IA pour qu'il ne soit pas
   rechargé depuis le disque à chaque question. <!-- claim:trbl-chat-slow-4 -->
4. **Réduisez la fenêtre de contexte** — une fenêtre plus petite signifie moins à traiter par
   tour, et c'est le premier réglage à baisser quand un modèle tient tout juste. <!-- claim:trbl-chat-slow-5 -->
5. **Désactivez le mode raisonnement**, qui échange de la vitesse contre de la précision. <!-- claim:trbl-chat-slow-6 -->

## Le GPU n'est pas utilisé (Windows / Linux) {#gpu-not-used}

<!-- claim:trbl-gpu-used-1 -->
Le journal de l'application indique sur quel périphérique un modèle a été chargé. Un
chargement GPU réussi ressemble à ceci :

```
llamacpp: chat model offload — Vulkan0 (Vulkan) has 15 GB free — offloading all layers
```

<!-- claim:trbl-gpu-used-2 -->
Si vous ne voyez pas une ligne de ce genre, le backend Vulkan n'a trouvé aucun périphérique
utilisable et s'est rabattu silencieusement sur le CPU — l'application fonctionne toujours,
mais plus lentement. Vérifiez, dans l'ordre :

1. **Votre pilote graphique.** C'est presque toujours la cause. Installez ou mettez à jour le
   pilote habituel de votre carte ; aucun kit CUDA ni SDK constructeur n'est nécessaire. <!-- claim:trbl-gpu-used-3 -->
2. **Que Vulkan voie le périphérique.** Lancez `vulkaninfo --summary` (paquet
   `vulkan-tools`). S'il ne signale aucun périphérique, le problème se situe sous EmailOps —
   corrigez d'abord la pile de pilotes. <!-- claim:trbl-gpu-used-4 -->
3. **La marge de VRAM.** Si le journal ne décharge qu'une *partie* des couches, le modèle est
   plus gros que la VRAM libre de la carte. Choisissez un modèle plus petit ou réduisez la
   fenêtre de contexte. <!-- claim:trbl-gpu-used-5 -->

<!-- claim:trbl-gpu-used-6 -->
Les machines virtuelles et les bureaux distants n'exposent souvent aucun GPU, ce qui est
normal.

## La recherche ne renvoie que des résultats par mots-clés {#search-returns-keyword-results-only}

<!-- claim:trbl-search-returns-1 -->
La recherche sémantique a besoin d'embeddings. Ouvrez **Paramètres → Recherche IA**, vérifiez
que les catégories qui vous intéressent sont sélectionnées et laissez la passe d'embeddings se
terminer. Après un changement de modèle d'embeddings, reconstruisez l'index depuis le même
écran.

<!-- claim:trbl-search-returns-2 -->
Vérifiez aussi **Limiter le traitement IA** dans les réglages d'IA — le courrier plus ancien que
cette fenêtre est délibérément ignoré.

## La classification n'étiquette rien {#classification-is-not-tagging-anything}

- Vérifiez que **Classer automatiquement les nouveaux courriels** est activé dans
  **Paramètres → Classification par IA**. <!-- claim:trbl-classification-tagging-1 -->
- Regardez quelles catégories Gmail sont sélectionnées ; si aucune ne l'est, rien n'est
  classé. <!-- claim:trbl-classification-tagging-2 -->
- Pour le courrier arrivé avant l'activation, utilisez **Classer les non classés**, ou
  **Tout reclasser** après une modification du prompt ou des règles. <!-- claim:trbl-classification-tagging-3 -->

## La synchronisation Gmail se bloque ou signale des limites

<!-- claim:trbl-gmail-sync-1 -->
Gmail impose des quotas par compte. Lorsqu'il demande à EmailOps de ralentir, la
synchronisation met ce compte en pause jusqu'à la réouverture de la fenêtre et reprend à la
prochaine exécution planifiée — aucune action requise. Si la synchronisation reste bloquée,
supprimez puis rajoutez le compte pour qu'un nouveau jeton soit émis.

## L'application est verrouillée et j'ai oublié le mot de passe principal

<!-- claim:trbl-app-locked-1 -->
Le mot de passe principal est un verrou local sans récupération possible — c'est précisément
le but. Votre courrier est toujours sur le serveur ; vous pouvez réinstaller EmailOps sur un
répertoire de données neuf et resynchroniser.

## Autre chose

<!-- claim:trbl-something-else-1 -->
Consultez les [tickets ouverts](https://github.com/emailops/emailops/issues) et, si votre
problème n'y figure pas, ouvrez-en un. Indiquez votre système d'exploitation et sa version, la
version d'EmailOps, le backend et le modèle d'IA utilisés, et ce que vous attendiez.
