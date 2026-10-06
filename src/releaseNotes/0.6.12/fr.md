### Nouveautés

- **Skills du chat (expérimental).** Enregistrez une procédure que le chat doit suivre pour un type de demande et activez-la dans **Paramètres → Skills IA**. Commencez un message par `/nom` pour l'utiliser.
- **Règles de pièces jointes suggérées.** EmailOps repère les pièces jointes récurrentes, comme les factures mensuelles, et propose des règles pour les collecter.
- **`/clear` dans le chat** démarre une nouvelle conversation.

### Confidentialité et sécurité

- OpenRouter n'achemine jamais votre courrier vers des fournisseurs qui s'entraînent dessus.
- La connexion à Gmail utilise PKCE, et la suppression d'un compte Gmail révoque l'accès d'EmailOps chez Google.
- Le mot de passe principal est temporisé après cinq tentatives erronées.

### Corrections

- La connexion à Gmail n'expire plus pendant les écrans de consentement de Google.
- Les brouillons IA affichent le gras, et les plages de dates du chat incluent leur dernier jour.
