---
title: 'Dépannage'
description: "Solutions aux problèmes les plus fréquents : IA indisponible, chat lent, recherche par mots-clés uniquement, erreurs de synchronisation."
weight: 60
---

## Les fonctions d'IA sont indisponibles

Avec le backend **intégré**, vérifiez que le modèle recommandé a fini de se télécharger dans
**Réglages → Backend et modèles d'IA**. Un téléchargement interrompu rend le modèle
inutilisable — supprimez-le et téléchargez-le à nouveau.

Si vous êtes passé à **Ollama**, assurez-vous que le démon tourne et est joignable sur
`http://localhost:11434`, et que vous avez récupéré un modèle :

```bash
ollama pull llama3.2
ollama pull nomic-embed-text
```

Sur un **Mac Intel**, le moteur intégré n'est pas inclus dans la version. Utilisez Ollama ou
OpenRouter.

## Le chat est lent

L'inférence locale prend un temps réel — sur une machine modeste, une réponse peut demander
des dizaines de secondes. Ce qui aide, à peu près par ordre d'efficacité :

1. **Vérifiez que le modèle tient vraiment.** C'est le point principal. Sous Windows ou Linux,
   un modèle plus grand que la **VRAM** de votre GPU déborde sur le CPU et devient plusieurs
   fois plus lent — la solution est un modèle plus petit, pas plus de RAM système. Sur Apple
   Silicon, la comparaison se fait avec la mémoire unifiée totale. Voir le
   [catalogue de modèles](../ai-features/#the-model-catalog) pour le chiffre de chaque modèle.
2. **Prenez un modèle plus petit.** Qwen 3.5 4B est le choix recommandé par défaut, et ce
   n'est pas un hasard.
3. **Augmentez « garder le modèle chargé »** dans les réglages d'IA pour qu'il ne soit pas
   rechargé depuis le disque à chaque question.
4. **Réduisez la fenêtre de contexte** — une fenêtre plus petite signifie moins à traiter par
   tour, et c'est le premier réglage à baisser quand un modèle tient tout juste.
5. **Désactivez le mode raisonnement**, qui échange de la vitesse contre de la précision.

## Le GPU n'est pas utilisé (Windows / Linux)

Le journal de l'application indique sur quel périphérique un modèle a été chargé. Un
chargement GPU réussi ressemble à ceci :

```
llamacpp: chat model offload — Vulkan0 (Vulkan) has 15 GB free — offloading all layers
```

Si vous ne voyez pas une ligne de ce genre, le backend Vulkan n'a trouvé aucun périphérique
utilisable et s'est rabattu silencieusement sur le CPU — l'application fonctionne toujours,
mais plus lentement. Vérifiez, dans l'ordre :

1. **Votre pilote graphique.** C'est presque toujours la cause. Installez ou mettez à jour le
   pilote habituel de votre carte ; aucun kit CUDA ni SDK constructeur n'est nécessaire.
2. **Que Vulkan voie le périphérique.** Lancez `vulkaninfo --summary` (paquet
   `vulkan-tools`). S'il ne signale aucun périphérique, le problème se situe sous EmailOps —
   corrigez d'abord la pile de pilotes.
3. **La marge de VRAM.** Si le journal ne décharge qu'une *partie* des couches, le modèle est
   plus gros que la VRAM libre de la carte. Choisissez un modèle plus petit ou réduisez la
   fenêtre de contexte.

Les machines virtuelles et les bureaux distants n'exposent souvent aucun GPU, ce qui est
normal.

## La recherche ne renvoie que des résultats par mots-clés

La recherche sémantique a besoin d'embeddings. Ouvrez **Réglages → Recherche IA**, vérifiez
que les catégories qui vous intéressent sont sélectionnées et laissez la passe d'embeddings se
terminer. Après un changement de modèle d'embeddings, reconstruisez l'index depuis le même
écran.

Vérifiez aussi la **limite d'ancienneté** dans les réglages d'IA — le courrier plus ancien que
cette fenêtre est délibérément ignoré.

## La classification n'étiquette rien

- Vérifiez que **classer automatiquement les nouveaux e-mails** est activé dans
  **Réglages → Classification IA**.
- Regardez quelles catégories Gmail sont sélectionnées ; si aucune ne l'est, rien n'est
  classé.
- Pour le courrier arrivé avant l'activation, utilisez **Classer les non classés**, ou
  **Tout reclasser** après une modification du prompt ou des règles.

## La synchronisation Gmail se bloque ou signale des limites

Gmail impose des quotas par compte. Lorsqu'il demande à EmailOps de ralentir, la
synchronisation met ce compte en pause jusqu'à la réouverture de la fenêtre et reprend à la
prochaine exécution planifiée — aucune action requise. Si la synchronisation reste bloquée,
supprimez puis rajoutez le compte pour qu'un nouveau jeton soit émis.

## L'application est verrouillée et j'ai oublié le mot de passe principal

Le mot de passe principal est un verrou local sans récupération possible — c'est précisément
le but. Votre courrier est toujours sur le serveur ; vous pouvez réinstaller EmailOps sur un
répertoire de données neuf et resynchroniser.

## Autre chose

Consultez les [tickets ouverts](https://github.com/emailops/emailops/issues) et, si votre
problème n'y figure pas, ouvrez-en un. Indiquez votre système d'exploitation et sa version, la
version d'EmailOps, le backend et le modèle d'IA utilisés, et ce que vous attendiez.
