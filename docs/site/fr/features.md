---
title: 'Fonctions standard'
description: "Le client e-mail lui-même : comptes, boîte unifiée, calendrier, pièces jointes, recherche et filtrage des indésirables."
weight: 30
---

Tout ce qui figure sur cette page fonctionne avec l'IA désactivée. La couche d'IA est traitée
séparément dans [Fonctions d'IA](../ai-features/).

## Comptes et synchronisation

Connectez autant de boîtes que vous voulez — Gmail, Outlook / Microsoft 365 (API Graph) et
n'importe quel serveur IMAP/SMTP (iCloud, Yahoo, Fastmail, ProtonMail Bridge, auto-hébergé).
Le courrier est synchronisé dans une base SQLite locale : la lecture et la recherche restent
rapides et fonctionnent hors ligne.

## Boîte de réception unifiée

La vue **Tous les comptes** fusionne chaque boîte activée en une seule liste, à côté des vues
par compte. Les dossiers IMAP personnalisés sont également synchronisés, et vous pouvez les
créer, les renommer, les supprimer et y déplacer des messages par glisser-déposer depuis
l'application.

## Filtres intelligents

Restreignez la liste par domaine, expéditeur ou étiquette de classification — pratique pour
traiter un client, un projet ou un déluge de newsletters à la fois.

## Calendrier

Vues mois, semaine et jour par compte pour Google Agenda et Outlook. Vous recevez des rappels
avant chaque événement, avec un bouton **Rejoindre** en un clic pour les liens Meet, Teams,
Webex et Zoom. La synchronisation du calendrier est active par défaut pour les comptes Gmail
et Outlook et peut être désactivée compte par compte, tout comme le délai de notification,
dans **Paramètres → Calendrier**.

Tous les agendas d'un compte sont synchronisés, pas seulement le principal — un agenda
qu'un collègue a partagé avec vous apparaît donc ici comme dans Google ou Outlook. Chacun
prend la couleur que lui donne son fournisseur, et la légende au-dessus de la grille masque
ou affiche les agendas un par un ; les mêmes interrupteurs se trouvent dans
**Paramètres → Calendrier**.

## Vue des pièces jointes

Un seul endroit qui liste toutes les pièces jointes de votre courrier — factures, contrats,
images — avec aperçu et export, au lieu de fouiller à nouveau les fils de discussion.

## Recherche

Recherche plein texte sur les objets, les corps, les expéditeurs et les pièces jointes. Avec
l'IA activée s'y ajoute la recherche sémantique, qui correspond au sens plutôt qu'aux mots
exacts.

## Indésirables et courrier de masse

EmailOps note localement chaque message entrant pour détecter le spam et le courrier de masse
non désiré. Aucun modèle ni appel réseau n'intervient, et vos corrections (« indésirable » /
« légitime ») entraînent le filtre au fil du temps. Vous décidez du sort du courrier signalé :

- **Les atténuer dans la liste** — ils restent en place, l'œil les saute simplement plus
  facilement.
- **Les sortir de la boîte de réception** — retirés de la liste, mais toujours accessibles
  par la recherche et dans les dossiers de votre fournisseur.

Aucune des deux options ne déplace ni ne supprime quoi que ce soit sur le serveur ; seul un
**Confirmer** explicite le fait. Un avertissement d'usurpation d'identité /
hameçonnage est proposé en option, désactivé par défaut.

## Contrôles de confidentialité et de sécurité

Un mot de passe principal verrouille l'application au démarrage, les images distantes et les
pixels de suivi sont bloqués jusqu'à autorisation, et les identifiants résident dans le
trousseau du système. Tout est détaillé dans
[Confidentialité et sécurité](../privacy-security/).

## Interface

Boîte en vue divisée ou pleine largeur, et une interface disponible en français, anglais,
espagnol et allemand. La langue de sortie de l'IA se règle séparément : vous pouvez lire
l'interface dans une langue et faire rédiger les réponses dans une autre.
