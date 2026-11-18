**Vous aimez EmailOps ?** Soutenez le projet avec une [étoile sur GitHub](https://github.com/emailops/emailops) : cela aide d'autres personnes à le découvrir.

### Nouveautés

- **Cci et objets modifiables** : chaque fenêtre de rédaction a un champ Cci, et l'objet d'une réponse ou d'un transfert peut être modifié avant l'envoi. Les réponses vont à l'adresse du Reply-To, comme celle d'une liste de diffusion.
- **Les brouillons conservent leurs pièces jointes** : un fichier est enregistré avec le brouillon dès que vous l'ajoutez, et les transferts sont aussi enregistrés comme brouillons.
- **Recherche booléenne** : des guillemets pour une expression exacte, `OR`, `-mot` ou `NOT` pour exclure un mot, et des parenthèses pour grouper. En plus, `cc:` et `has:attachment` pour filtrer.
- **Le nouveau courrier est prêt plus tôt pour la recherche IA** : les Embeddings sont générés du plus récent au plus ancien, sur tous vos comptes.

### Corrections

- IMAP : un message supprimé n'apparaît qu'une fois dans Supprimés, et sur Proton Mail Bridge il n'est plus effacé définitivement.
- IMAP : les comptes se connectent même quand la première adresse du serveur ne répond pas.
- IMAP : sur les serveurs anciens, déplacer un message n'efface plus ceux qu'une autre app a marqués pour suppression.
- IMAP : une réponse du serveur qu'EmailOps ne sait pas lire n'arrête plus la synchronisation.
- Les brouillons IA suivent vos instructions avec n'importe quel modèle, et une longue consigne n'est plus coupée sans prévenir.
- Les pièces jointes Gmail conservées avec le message s'ouvrent à nouveau.
- Un brouillon refusé par Gmail n'est plus renvoyé chaque minute.
