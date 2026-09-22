---
title: 'Fonctions standard'
description: "Le client e-mail lui-même : comptes, boîte unifiée, calendrier, pièces jointes, recherche et filtrage des indésirables."
weight: 30
nav:
  unified-inbox: view/inbox
  calendar: view/calendar
  attachments-view: view/attachments
  junk-and-bulk-mail: settings/junk
  privacy-and-security-controls: settings/privacy
  interface: settings/appearance
---

Tout ce qui figure sur cette page fonctionne avec l'IA désactivée. La couche d'IA est traitée
séparément dans [Fonctions d'IA](../ai-features/).

## Comptes et synchronisation

Connectez autant de boîtes que vous voulez — Gmail, Outlook / Microsoft 365 (API Graph) et
n'importe quel serveur IMAP/SMTP (iCloud, Yahoo, Fastmail, ProtonMail Bridge, auto-hébergé).
Le courrier est synchronisé dans une base SQLite locale : la lecture et la recherche restent
rapides et fonctionnent hors ligne.

Le nom d'un compte est le nom d'expéditeur que voient les destinataires des e-mails envoyés
depuis ce compte. Modifiez-le dans le champ **Nom de l'expéditeur** des réglages du compte, ou
laissez-le vide pour envoyer avec l'adresse seule. Les comptes Gmail reprennent au départ le
nom du paramètre « Envoyer des e-mails en tant que » de Gmail, et les comptes IMAP le nom
affiché indiqué lors de leur connexion. Les e-mails envoyés via Outlook portent le nom que
Microsoft associe à la boîte.

## Boîte de réception unifiée {#unified-inbox}

La vue **Tous les comptes** fusionne chaque boîte activée en une seule liste, à côté des vues
par compte. Les dossiers IMAP personnalisés sont également synchronisés, et vous pouvez les
créer, les renommer, les supprimer et y déplacer des messages par glisser-déposer depuis
l'application.

## Transférer

**Transférer** se trouve à côté de **Répondre** et **Répondre à tous** dans le volet de
lecture. Le brouillon s'ouvre sans destinataire et contient le message d'origine sous un
en-tête *Message transféré* avec son expéditeur, sa date et ses destinataires, ainsi que les
pièces jointes d'origine (jusqu'à 20 Mo au total). Il part comme un nouveau message et ne
rejoint donc pas les conversations existantes du destinataire.

## Filtres intelligents

Restreignez la liste par domaine, expéditeur ou étiquette de classification — pratique pour
traiter un client, un projet ou un déluge de newsletters à la fois. Avec l'IA activée, ces
mêmes étiquettes alimentent aussi le [Tableau d'étiquettes](../ai-features/#tag-board), qui
les présente sous forme de grille de blocs.

## Calendrier {#calendar}

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

## Vue des pièces jointes {#attachments-view}

Un seul endroit pour les pièces jointes qui comptent — factures, contrats, reçus — avec aperçu
et téléchargement, au lieu de fouiller à nouveau les fils de discussion. Ouvrez-la depuis
**Pièces jointes** dans la barre latérale.

La vue collecte les pièces jointes grâce à des **règles**, elle est donc vide au départ. Cliquez
sur **Gérer les règles** (ou **Créer une règle** dans la vue vide) et remplissez :

- **Nom de la règle** — le nom affiché dans la liste.
- **Motif de l'expéditeur** — séparés par des virgules ; correspondance exacte sauf s'il contient
  `*` (`*apple.com*` correspond à tout expéditeur contenant « apple.com »). Laissez vide pour
  n'importe quel expéditeur.
- **Motif de l'objet** et **Motif du nom de fichier** — `*` est un joker ; seuls les noms de
  fichiers correspondants sont collectés.
- **Étiquettes** — séparées par des virgules ; elles apparaissent comme boutons de filtre en haut
  de la vue.

Tous les motifs renseignés doivent correspondre. Les règles s'appliquent au nouveau courrier au
fil de la synchronisation ; cochez **Appliquer aux e-mails existants après la création** pour
collecter aussi dans le courrier déjà présent. Sélectionnez des pièces jointes pour les
télécharger ensemble dans votre dossier Téléchargements.

## Recherche

Recherche plein texte sur les objets, les corps, les expéditeurs et les pièces jointes. Avec
l'IA activée s'y ajoute la recherche sémantique, qui correspond au sens plutôt qu'aux mots
exacts.

## Indésirables et courrier de masse {#junk-and-bulk-mail}

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

## Contrôles de confidentialité et de sécurité {#privacy-and-security-controls}

Un mot de passe principal verrouille l'application au démarrage, les images distantes et les
pixels de suivi sont bloqués jusqu'à autorisation, et les identifiants résident dans le
trousseau du système. Tout est détaillé dans
[Confidentialité et sécurité](../privacy-security/).

## Interface {#interface}

Boîte en vue divisée ou pleine largeur, et une interface disponible en français, anglais,
espagnol et allemand. La langue de sortie de l'IA se règle séparément : vous pouvez lire
l'interface dans une langue et faire rédiger les réponses dans une autre.
