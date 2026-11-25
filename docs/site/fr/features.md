---
title: 'Fonctions standard'
description: "Le client e-mail lui-même : comptes, boîte unifiée, archives, mise en attente, envoi programmé, signatures, calendrier, pièces jointes, recherche, filtrage des indésirables, notifications et raccourcis clavier."
weight: 30
nav:
  unified-inbox: view/inbox
  calendar: view/calendar
  attachments-view: view/attachments
  junk-and-bulk-mail: settings/junk
  privacy-and-security-controls: settings/privacy
  interface: settings/appearance
  undo-send-and-scheduled-send: settings/appearance
  unsubscribe-and-block-sender: settings/junk
  keyboard-shortcuts: settings/appearance
---

<!-- claim:feat-intro-1 -->
Tout ce qui figure sur cette page fonctionne avec l'IA désactivée. La couche d'IA est traitée
séparément dans [Fonctions d'IA](../ai-features/).

## Comptes et synchronisation

<!-- claim:feat-accounts-sync-1 -->
Connectez autant de boîtes que vous voulez — Gmail, Outlook / Microsoft 365 (API Graph) et
n'importe quel serveur IMAP/SMTP (iCloud, Yahoo, Fastmail, ProtonMail Bridge, auto-hébergé).
Le courrier est synchronisé dans une base SQLite locale : la lecture et la recherche restent
rapides et fonctionnent hors ligne.

<!-- claim:feat-accounts-sync-2 -->
Le nom d'un compte est le nom d'expéditeur que voient les destinataires des e-mails envoyés
depuis ce compte. Modifiez-le dans le champ **Nom de l'expéditeur** des réglages du compte, ou
laissez-le vide pour envoyer avec l'adresse seule. Les comptes Gmail reprennent au départ le
nom du paramètre « Envoyer des e-mails en tant que » de Gmail, et les comptes IMAP le nom
affiché indiqué lors de leur connexion. Les e-mails envoyés via Outlook portent le nom que
Microsoft associe à la boîte.

<!-- claim:feat-accounts-default-sender -->
**Paramètres → Apparence → Envoyer les nouveaux messages depuis** choisit le compte avec lequel
commence un nouveau message, quel que soit le compte affiché dans la barre latérale ; avec
**Compte de la vue sélectionnée**, un nouveau message part du compte que vous consultez. Les
réponses et les transferts gardent le compte de leur message, et les brouillons le compte dans
lequel ils ont été enregistrés. Chaque liste **De** place en tête le compte depuis lequel vous
envoyez le plus d'e-mails, compté sur votre ordinateur.

## Boîte de réception unifiée {#unified-inbox}

<!-- claim:feat-unified-inbox-1 -->
La vue **Tous les comptes** fusionne chaque boîte activée en une seule liste, à côté des vues
par compte. Les dossiers IMAP personnalisés sont également synchronisés, et vous pouvez les
créer, les renommer, les supprimer et y déplacer des messages par glisser-déposer depuis
l'application.

## Répondre

<!-- claim:reading-pane-reply -->
**Répondre** s'adresse à l'expéditeur, ou à l'adresse que le message indique dans son en-tête
*Reply-To*, comme le font les listes de diffusion ou l'adresse *no-reply* d'une boutique.
**Répondre à tous** ajoute tous les autres participants de la conversation. L'objet d'une
réponse ou d'un transfert se modifie avant l'envoi, et la réponse reste dans sa conversation.
Chaque fenêtre de rédaction propose **Cc** et **Cci** : les copies cachées parviennent à leurs
destinataires sans que les autres les voient, et un message peut partir uniquement en Cc ou en
Cci.

<!-- claim:compose-drafts-keep-files -->
Ce que vous écrivez est enregistré comme brouillon au fil de la saisie, pièces jointes
comprises : fermer la fenêtre de rédaction ou l'application le conserve, et un transfert est
conservé aussi. Un message contient jusqu'à 32 fichiers de 25 Mo au plus chacun.

<!-- claim:compose-drafts-imap-folder -->
Sur un compte IMAP, les brouillons qu'un autre logiciel de messagerie a enregistrés dans le
dossier des brouillons du compte apparaissent aussi dans **Brouillons**, avec leurs
destinataires, leur objet, leur texte ou leur mise en forme, leurs pièces jointes et la
conversation à laquelle ils répondent, et ils suivent les modifications qui y sont faites.
EmailOps ne fait que lire ce dossier : un brouillon modifié ici garde vos changements sur cet
ordinateur, et un brouillon supprimé ou envoyé ici reste dans le dossier pour vos autres
logiciels de messagerie.

## Transférer

<!-- claim:reading-pane-forward -->
**Transférer** se trouve à côté de **Répondre** et **Répondre à tous** dans le volet de
lecture. Le brouillon s'ouvre sans destinataire et contient toute la conversation, du message
le plus ancien au plus récent : chaque message sous son propre en-tête *Message transféré*
avec son expéditeur, sa date et ses destinataires, sans l'historique qu'un message précédent
montre déjà, ainsi que les pièces jointes d'origine (jusqu'à 20 Mo au total). Il part comme un nouveau message et ne
rejoint donc pas les conversations existantes du destinataire.

## Organiser les conversations {#organizing-conversations}

<!-- claim:feat-organize-1 -->
**Archiver** sort une conversation de la boîte de réception sans la supprimer, et **Déplacer vers
la boîte de réception** l'y remet. Ces deux actions, ainsi que **Marquer comme non lu** et
**Ajouter aux favoris**, se trouvent dans le volet de lecture et dans le menu **Plus d'actions**
(⋮) de chaque conversation ; l'étoile figure aussi sur chaque ligne de la liste. Le changement est
également appliqué chez votre fournisseur de messagerie, si bien que Gmail ou Outlook affichent la
même chose.

<!-- claim:feat-organize-2 -->
**Favoris**, dans la barre latérale, liste vos conversations marquées d'une étoile. **Archives**
liste le courrier archivé des comptes Gmail et Outlook, ainsi que dans **Tous les comptes** ; un
compte IMAP archive dans son propre dossier d'archives, qui apparaît avec ses autres dossiers. Le
courrier archivé quitte seulement la boîte de réception : la recherche, les filtres intelligents
et les fonctions d'IA y ont toujours accès.

<!-- claim:feat-list-sent-rows -->
Dans toutes les listes, la ligne d'un message que vous avez envoyé indique à qui il était
adressé (« À : Ana, Luis +1 ») au lieu de votre propre nom, et son info-bulle liste tous les
destinataires. Le nom de chaque destinataire vient du message lui-même ou de la façon dont cette
personne signe quand elle vous écrit ; à défaut, la ligne affiche l'adresse.

<!-- claim:feat-organize-3 -->
Cochez la case au début d'une ligne pour la sélectionner. Dès qu'une ou plusieurs lignes sont
sélectionnées, une barre au-dessus de la liste agit sur toutes à la fois — archiver, mettre en
attente, supprimer, marquer comme lu ou non lu, ajouter aux favoris — et
**Effacer la sélection** y met fin. Les comptes qui ont leurs propres dossiers proposent aussi
**Déplacer vers un dossier**.

<!-- claim:feat-organize-4 -->
Archiver ou supprimer retire aussitôt les conversations de la liste et affiche un avis avec
**Annuler** pendant 6 secondes. Votre fournisseur de messagerie n'est prévenu qu'une fois ces
secondes écoulées (ou plus tôt, si vous archivez ou supprimez autre chose ou ouvrez une autre
vue) : Annuler se contente donc de les remettre en place.

<!-- claim:feat-organize-undo-window -->
**Paramètres → Apparence → Annuler l’archivage et la suppression** règle ce délai : désactivé, 6,
10, 20 ou 30 secondes, 6 par défaut. Désactivé, l'archivage et la suppression parviennent aussitôt
à votre fournisseur de messagerie, sans Annuler. L’annulation proposée après une mise en attente
dure aussi longtemps. **Annuler l’envoi** est un réglage distinct.

<!-- claim:feat-organize-delete-forever -->
Dans **Corbeille** et **Indésirables**, **Supprimer définitivement** dans la barre de sélection
efface les messages sélectionnés, et **Vider la corbeille** ou **Vider les indésirables** au-dessus
de la liste les efface tous. Vous voyez d'abord combien de messages vont partir et vous
confirmez ; une barre montre la progression, **Annuler** s'arrête après le lot en cours, et la fin
indique combien ont été effacés et pourquoi certains n'ont pas pu l'être. Seul ce dossier est
touché. Cela fonctionne sur les comptes IMAP et Outlook. Gmail efface de lui-même ce qui est dans
la corbeille ou les indésirables depuis 30 jours ; effacer plus tôt depuis EmailOps demanderait
un accès complet au compte Gmail, qu'EmailOps ne demande pas.

<!-- claim:feat-organize-5 -->
Quand la conversation que vous lisez quitte la liste — archivée, supprimée, mise en attente ou
déplacée dans les spams —, la suivante s'ouvre. **Paramètres → Apparence → Après l’archivage ou
la suppression** permet de choisir entre la conversation suivante, la précédente ou le retour à
la liste. Marquer une conversation comme non lue ramène toujours à la liste.

<!-- claim:feat-organize-6 -->
**Tous** et **Non lus**, en haut de la liste, choisissent ce qu'elle affiche. **Non lus** ne garde
que les conversations qui ont un message non lu — dans les vues qui listent des messages
isolés, comme le courrier envoyé et les dossiers, seulement les messages non lus — dans toutes
les vues, avec un filtre intelligent et pendant une recherche. Une
conversation que vous ouvrez reste dans la liste jusqu'à son rechargement, et le choix dure
jusqu'à ce que vous quittiez EmailOps.

## Mettre en attente {#snooze}

<!-- claim:feat-snooze-1 -->
**Mettre en attente** masque une conversation de la boîte de réception jusqu'au moment choisi :
plus tard aujourd'hui, demain, ce week-end, la semaine prochaine, ou la date et l'heure de votre
choix. L'action est proposée dans le volet de lecture, dans le menu ⋮ de la ligne et dans la barre
de sélection.

<!-- claim:feat-snooze-2 -->
Les conversations en attente sont listées sous **En attente** dans la barre latérale, la plus
proche en premier, où **Annuler la mise en attente** en fait revenir une plus tôt. Le moment venu,
la conversation revient en haut de la boîte de réception, marquée comme non lue. Un nouveau
message dans une conversation en attente la fait revenir immédiatement.

<!-- claim:feat-snooze-3 -->
La mise en attente n'est conservée que sur cet ordinateur : les autres applications de messagerie
continuent d'afficher la conversation dans la boîte de réception. Les conversations reviennent
tant qu'EmailOps est ouvert ; une conversation dont l'heure est passée pendant que l'application
était fermée revient à sa prochaine ouverture.

## Annuler l'envoi et envoi programmé {#undo-send-and-scheduled-send}

<!-- claim:feat-send-1 -->
Après un clic sur **Envoyer**, le message attend quelques secondes avec un avis **Annuler** ;
Annuler le reprend et le rouvre pour modification. Le délai se règle dans **Paramètres →
Apparence → Annuler l’envoi** : désactivé, 5, 10, 20 ou 30 secondes, 10 par défaut. EmailOps doit
rester ouvert jusqu'à ce que le message soit parti.

<!-- claim:feat-send-2 -->
La flèche à côté d'**Envoyer** ouvre **Programmer l’envoi** : demain matin, demain après-midi,
lundi matin, ou la date et l'heure de votre choix.

<!-- claim:feat-send-3 -->
Les messages en attente d'envoi sont listés sous **Programmés** dans la barre latérale, où vous
pouvez **Envoyer maintenant**, **Modifier** ou **Supprimer** chacun d'eux. Un message programmé ne
part que si EmailOps est ouvert ; un message dont l'heure est passée pendant que l'application
était fermée est envoyé à sa prochaine ouverture. Un message qui n'a pas pu être envoyé y reste,
marqué **Non envoyé**, avec **Réessayer** : EmailOps ne le renvoie jamais de lui-même.
## Signatures {#signatures}

<!-- claim:feat-signatures-1 -->
Chaque compte a sa propre signature, définie dans **Paramètres → Signatures**. Deux interrupteurs
décident où elle va : **Insérer dans les nouveaux messages** et **Insérer dans les réponses et
transferts**. Dans un nouveau message ou une réponse, elle se place sous votre texte ; dans un
transfert, au-dessus du message transféré. Elle fait partie du corps du message : vous pouvez donc
la modifier ou la supprimer dans n'importe quel message avant l'envoi.

<!-- claim:feat-signatures-2 -->
**Ajouter une image** y insère un logo ou une image de votre signature manuscrite. PNG, JPEG, GIF
et WebP sont acceptés ; SVG et les autres fichiers sont refusés, avec la raison. Une image large
est réduite à 600 px, chaque image peut peser 200 Ko au plus et la signature entière 512 Ko. Les
comptes Gmail proposent aussi **Importer depuis Gmail**, qui copie dans l'éditeur la signature que
Gmail a pour cette adresse.

<!-- claim:feat-signatures-3 -->
Dans la version texte brut d'un message, une signature qui le termine est précédée de la ligne
standard `-- `, pour que les autres applications de messagerie la reconnaissent. Quand le compte a
une signature pour ce type de message, les brouillons de l'IA omettent votre nom et vos
coordonnées et laissent la signature signer.

<!-- claim:feat-signatures-4 -->
Sous la signature, **Ajouter « Envoyé avec EmailOps » à la fin des messages que j'envoie**
décide si les messages que vous envoyez se terminent par cette ligne. Elle est activée par
défaut et s'applique à tous les comptes ; les messages déjà envoyés gardent leur pied de message.

## Filtres intelligents

<!-- claim:feat-smart-filters-1 -->
Restreignez la liste par domaine, expéditeur ou étiquette de classification — pratique pour
traiter un client, un projet ou un déluge de newsletters à la fois. Avec l'IA activée, ces
mêmes étiquettes alimentent aussi le [Tableau d'étiquettes](../ai-features/#tag-board), qui
les présente sous forme de grille de blocs.

<!-- claim:feat-smart-filters-2 -->
Cliquer sur un filtre l'écrit dans le champ de recherche sous forme de requête visible et
modifiable — `tag:priority=urgent`, `from:`, `domain:`, `ext:` — et un second clic l'efface.
Clic droit sur un filtre puis **Ajouter à la recherche** le combine avec la recherche en cours.
Un filtre ou une recherche s'applique dans la vue où vous êtes : les éléments envoyés, le spam,
la corbeille et les dossiers ne listent que leur courrier, et la boîte de réception atteint tout
sauf le spam et la corbeille. La recherche reste active quand vous changez de compte ; si elle ne
trouve rien, la liste indique qu'une recherche est active et propose de l'effacer. Chaque groupe d'étiquettes affiche d'abord ses dix valeurs les plus
utilisées, avec un bouton en dessous pour voir le reste.

## Calendrier {#calendar}

<!-- claim:feat-calendar-1 -->
Vues mois, semaine et jour pour Google Agenda et Outlook, d'un compte ou de **Tous les
comptes** ensemble, où chaque événement prend la couleur de son compte. Vous recevez des rappels
avant chaque événement, avec un bouton **Rejoindre** en un clic pour les liens Meet, Teams,
Webex et Zoom. La synchronisation du calendrier est active par défaut pour les comptes Gmail
et Outlook et peut être désactivée compte par compte, tout comme le délai de notification,
dans **Paramètres → Calendrier**.

<!-- claim:feat-calendar-2 -->
Tous les agendas d'un compte sont synchronisés, pas seulement le principal — un agenda
qu'un collègue a partagé avec vous apparaît donc ici comme dans Google ou Outlook. Chacun
prend la couleur que lui donne son fournisseur, et la légende au-dessus de la grille masque
ou affiche les agendas un par un ; les mêmes interrupteurs se trouvent dans
**Paramètres → Calendrier**. Les rappels de réunion ne viennent que de vos propres agendas :
un agenda qu'une autre personne a partagé avec vous affiche ses réunions sans vous les
rappeler. Une réunion à laquelle vous êtes invité reçoit toujours un seul rappel.

<!-- claim:feat-calendar-4 -->
Les comptes IMAP ont un calendrier en s'abonnant à un lien de calendrier publié (ICS), par
exemple l'URL de partage par lien de Proton Calendar, dans **Flux de calendrier (comptes IMAP)**
sous **Paramètres → Calendrier**. Un flux est en lecture seule, se met à jour toutes les quelques
minutes, et son lien est conservé dans votre trousseau, pas dans la base de données.

<!-- claim:feat-calendar-5 -->
À côté de la boîte de réception, le calendrier s'ouvre aussi en panneau ancré à droite,
l'**Agenda** ; **Ouvrir le calendrier complet** passe à la vue complète.

<!-- claim:feat-calendar-6 -->
**Importer un .ics** dans la barre d'outils du calendrier ouvre un sélecteur de fichier pour un fichier `.ics`, puis un aperçu qui range chaque événement sous **Sera importé**, **Déjà dans le calendrier** ou **Ne peut pas être importé**, avec la raison pour ce dernier cas (par exemple, une règle de répétition qu'Outlook ne sait pas représenter). Choisissez le compte sous **Importer dans** (Gmail ou Outlook) et les événements vont dans le calendrier principal de ce compte. Un fichier peut peser jusqu'à 10 Mo et contenir 500 événements, et réimporter le même fichier ne crée pas de doublons. Les comptes IMAP ne sont pas proposés, car leurs calendriers sont des flux en lecture seule, et il n'est pas possible de choisir un calendrier secondaire.

<!-- claim:feat-calendar-7 -->
Une invitation dans un message affiche une carte avec **Oui**, **Non** et **Peut-être**. Si l'invitation n'est pas encore dans votre calendrier, **Oui** ou **Peut-être** l'ajoute au calendrier du compte qui a reçu le message et envoie votre réponse à l'organisateur par e-mail ; la carte indique « Votre réponse sera envoyée à » l'organisateur, avec un avertissement lorsque l'organisateur n'est pas l'expéditeur du message. **Non** supprime une copie ajoutée par EmailOps et envoie le refus ; pour une seule date d'une série, il envoie seulement le refus. Si l'organisateur déplace une réunion ajoutée par EmailOps, répondre à l'invitation mise à jour déplace la copie au nouvel horaire ; répondre à un message plus ancien ne la remet pas à l'ancien horaire, et une série récurrente n'est pas déplacée. Si l'invitation est arrivée il y a moins de dix minutes, la carte vous demande de réessayer dans un instant. Les billets et webinaires sans organisateur affichent à la place un seul bouton **Ajouter au calendrier**.

<!-- claim:feat-calendar-8 -->
Dans le détail d'un événement, les icônes d'enveloppe **Écrire à l'organisateur** et **Écrire à tous les participants** ouvrent un nouveau message depuis le compte de l'événement, avec le titre de l'événement comme objet. Lorsque vous êtes invité, une ligne **Votre réponse** (**Oui**, **Non**, **Peut-être**) modifie votre réponse ; pour un événement récurrent, vous choisissez **Uniquement cet événement** ou **Tous les événements**.

<!-- claim:feat-calendar-3 -->
Les notifications système d'un rappel omettent le titre de la réunion, sauf si vous activez
**Afficher le titre de la réunion dans les notifications** dans **Paramètres → Calendrier** :
elles peuvent apparaître sur l'écran verrouillé et dans le centre de notifications, hors du
verrouillage de l'app. Le rappel dans EmailOps affiche toujours la réunion complète.

## Vue des pièces jointes {#attachments-view}

<!-- claim:feat-attachments-view-1 -->
Un seul endroit pour les pièces jointes qui comptent — factures, contrats, reçus — avec aperçu
et téléchargement, au lieu de fouiller à nouveau les fils de discussion. Ouvrez-la depuis
**Pièces jointes** dans la barre latérale.

<!-- claim:feat-attachments-view-9 -->
Un PDF s'ouvre dans son propre onglet, où **Zoom avant** et **Zoom arrière** changent sa taille.
Un document Word (.docx) s'ouvre lui aussi dans son propre onglet, en lecture seule.

<!-- claim:feat-attachments-view-2 -->
La vue collecte les pièces jointes grâce à des **règles**, elle est donc vide au départ. Cliquez
sur **Gérer les règles** (ou **Créer une règle** dans la vue vide) et remplissez :

- **Nom de la règle** — le nom affiché dans la liste. <!-- claim:feat-attachments-view-3 -->
- **Motif de l'expéditeur** — séparés par des virgules ; correspondance exacte sauf s'il contient
  `*` (`*apple.com*` correspond à tout expéditeur contenant « apple.com »). Laissez vide pour
  n'importe quel expéditeur. <!-- claim:feat-attachments-view-4 -->
- **Motif de l'objet** et **Motif du nom de fichier** — `*` est un joker ; seuls les noms de
  fichiers correspondants sont collectés. <!-- claim:feat-attachments-view-5 -->
- **Étiquettes** — choisissez celles que vous utilisez déjà ou saisissez-en une nouvelle ; elles
  apparaissent comme boutons de filtre en haut de la vue. <!-- claim:feat-attachments-view-6 -->

<!-- claim:feat-attachments-view-7 -->
Tous les motifs renseignés doivent correspondre. Les règles s'appliquent au nouveau courrier au
fil de la synchronisation ; cochez **Appliquer aux e-mails existants après la création** pour
collecter aussi dans le courrier déjà présent. Les règles s'appliquent à la boîte de réception, aux
Éléments envoyés, aux archives et à vos propres dossiers, jamais au Spam ni à la Corbeille. Sélectionnez des
pièces jointes pour les télécharger ensemble dans votre dossier Téléchargements.

<!-- claim:feat-attachments-view-8 -->
EmailOps propose aussi des règles de lui-même. Quand un même expéditeur vous envoie régulièrement
des documents (PDF, fichiers Office ou factures électroniques) — au moins deux e-mails sur deux mois
différents, dans la boîte de réception ou dans un dossier où vous les classez —, une section **Règles suggérées** apparaît dans **Gérer les règles**, et un
compteur à côté de **Pièces jointes** dans la barre latérale indique leur nombre. **Examiner**
ouvre le formulaire de la règle déjà rempli (expéditeur et motif de nom de fichier) ; la
règle n'est créée que lorsque vous l'enregistrez. **Ignorer** masque la suggestion définitivement,
même si l'expéditeur écrit plus tard depuis une autre adresse ; **Annuler**, ou **Restaurer** dans
**Suggestions ignorées**, la fait revenir. Les e-mails de votre propre adresse
ou de collègues de votre propre entreprise ne sont jamais suggérés.

<!-- claim:feat-attachments-view-10 -->
Avec **Tous les comptes** sélectionné dans la barre latérale, la vue réunit dans une seule liste les
pièces jointes de tous les comptes activés, et chaque ligne indique son compte. **Tout
sélectionner** coche toutes les pièces jointes du filtre en cours, pas seulement les lignes déjà
chargées. Si les fichiers que vous téléchargez viennent de plusieurs comptes, chaque nom de fichier
commence par l'adresse de son compte. Chaque règle appartient à un compte et collecte le courrier
de ce compte. Avec **Tous les comptes** sélectionné, **Gérer les règles** affiche les règles et
suggestions de tous les comptes activés, chacune avec son compte, et une nouvelle règle va dans le
compte que vous choisissez : celui de l'e-mail si vous l'ouvrez depuis **Créer une règle de pièce
jointe** dans le menu d'un e-mail, sinon le premier compte activé. Les étiquettes sont communes à
tous les comptes : le champ d'étiquettes d'une règle propose celles de tous les comptes. Le champ
d'expéditeur suggère, pendant la saisie, les expéditeurs du compte de la règle, votre propre adresse
comprise. **Ne
garder que la dernière version de chaque fichier d'un fil**, activée par défaut, conserve une seule
copie d'un document qui fait des allers-retours : un fichier du même nom renvoyé dans la même
conversation remplace la copie précédente. Pour collecter les
factures que vous envoyez, créez la règle dans le compte d'envoi, avec votre propre adresse comme
expéditeur et un motif de nom de fichier comme `*facture*`.

## EO Docs {#eo-docs}

<!-- claim:feat-eo-docs-1 -->
**EO Docs** vous permet d'écrire des documents et des feuilles de calcul avec d'autres utilisateurs
d'EmailOps, sans cloud entre vous : chaque modification voyage comme un e-mail ordinaire entre vos
comptes, et chaque copie fusionne ce qui arrive sans conflit. C'est expérimental et activé par
défaut ; **Paramètres → EO Docs** le désactive, et tant qu'il est désactivé rien n'est reçu ni envoyé.

<!-- claim:feat-eo-docs-2 -->
Ouvrez **EO Docs** dans la barre latérale et cliquez sur **Nouveau** pour créer un document ou une
feuille. Les documents ont des titres, du gras, de l'italique, du soulignement, des listes, des
liens, des tableaux et des images. Les feuilles s'agrandissent en lignes et en colonnes, acceptent
un bloc collé depuis Excel, et une colonne s'élargit en faisant glisser le bord de son en-tête.
Annuler et rétablir ne reprennent que vos propres modifications.

<!-- claim:feat-eo-docs-3 -->
Une cellule qui commence par `=` est une formule : `SUM`, `AVERAGE`, `MIN`, `MAX` et `COUNT`
(ou `SUMA`, `PROMEDIO` et `CONTAR`) sur des plages comme `=SUM(B2:B10)`. Insérer ou supprimer des
lignes garde les plages sur les mêmes cellules. Le bouton de filtre d'un en-tête de colonne masque
les lignes que vous décochez ; les filtres ne changent que votre propre vue.

<!-- claim:feat-eo-docs-9 -->
Les feuilles sont basiques pour l'instant : texte, nombres et ces formules, sans mise en forme,
graphiques ni autres fonctions. Une note au-dessus de chaque feuille le rappelle jusqu'à ce que
vous la fermiez.

<!-- claim:feat-eo-docs-8 -->
Quand deux personnes modifient la même cellule avant d'avoir vu la modification de l'autre,
toutes les copies gardent la même des deux valeurs. La cellule est alors signalée, un avis indique
la valeur perdue, et vous choisissez : la rétablir ou garder celle affichée. **Historique** liste
aussi ces cellules, réglées ou non.

<!-- claim:feat-eo-docs-4 -->
**Partager** demande les adresses e-mail, en proposant d'abord les collègues de votre entreprise, et
votre consentement : dès lors, EmailOps leur envoie vos modifications de lui-même, environ deux
minutes après que vous avez cessé d'écrire, ou tout de suite avec **Envoyer les modifications**. Les
autres utilisateurs d'EmailOps reçoivent une invitation à **Accepter** ; les autres reçoivent une
copie en lecture seule dans l'invitation. Les modifications arrivent à la synchronisation suivante,
sont fusionnées, et leurs e-mails sont marqués comme lus et archivés.

<!-- claim:feat-eo-docs-5 -->
Une modification n'est appliquée que si elle vient d'une personne avec qui le document est partagé.
Sur les comptes Gmail et Outlook, elle est aussi refusée si l'expéditeur échoue au contrôle
d'authentification de votre fournisseur (DMARC, ou SPF sans signature DKIM valide). Les e-mails ne
sont pas chiffrés de bout en bout : ils sont aussi privés que le reste de votre courrier.

<!-- claim:feat-eo-docs-6 -->
Vos propres dossiers (jamais partagés) rangent les documents ; faites glisser un document sur un
dossier, ou utilisez **Déplacer vers**. La recherche trouve les documents par titre et par contenu,
et **Historique** montre les versions précédentes. **Exporter en PDF** enregistre le document
en PDF dans votre dossier Téléchargements. **Supprimer** demande d'abord confirmation ;
supprimer un document partagé n'efface que votre copie, et les autres gardent la leur.

<!-- claim:feat-eo-docs-7 -->
**Importer** transforme un document Word (`.docx`) ou une feuille de calcul (`.xlsx`)
en EO Docs, une feuille par onglet et les formules sous forme de valeurs ; **Ouvrir dans EO Docs**
fait de même avec une pièce jointe. Dans l'éditeur d'e-mails, **Depuis EO Docs** joint un document,
ce qui le partage avec les destinataires de l'e-mail.

## Recherche

<!-- claim:feat-search-1 -->
Recherche plein texte sur les objets, les corps et les expéditeurs. La liste va jusqu'au dernier
résultat du courrier téléchargé, et le courrier signalé par le détecteur apparaît avec son avertissement.

<!-- claim:feat-search-2 -->
Les recherches se précisent avec des opérateurs, seuls ou à côté de texte libre :

| Opérateur | Recherche |
|---|---|
| `from:ana` | adresse ou nom de l'expéditeur |
| `to:ana` | destinataire |
| `cc:ana` | une personne en copie (Cc) |
| `subject:facture` | objet |
| `before:2026-09-01` / `after:2026-09-01` | date de réception |
| `id:<id du courriel>` | un courriel précis |
| `tag:newsletter` / `tag:intent=request` | une étiquette du classifieur, éventuellement dans une facette |
| `domain:example.com` | le domaine de l'expéditeur |
| `ext:pdf` | une pièce jointe de ce type de fichier |
| `has:attachment` / `-has:attachment` | courriels avec / sans pièce jointe |

<!-- claim:feat-search-3 -->
Dans le texte libre, une expression entre guillemets cherche ses mots dans cet ordre, `OR`
entre deux mots trouve l'un ou l'autre, un `-` (ou `NOT`) devant un mot écarte les courriels
qui le contiennent, et les parenthèses regroupent : `(facture OR reçu) -brouillon`. Les mots
côte à côte doivent tous être présents et se regroupent avant `OR` : `facture OR reçu mars`
veut dire facture, ou reçu et mars. Les opérateurs s'écrivent en majuscules ; un « or » en
minuscules est un mot ordinaire.

## Indésirables et courrier de masse {#junk-and-bulk-mail}

<!-- claim:feat-junk-bulk-1 -->
EmailOps note localement chaque message entrant pour détecter le spam et le courrier de masse
non désiré. Aucun modèle ni appel réseau n'intervient, et vos corrections (« indésirable » /
« légitime ») entraînent le filtre au fil du temps. Vous décidez du sort du courrier signalé :

- **Les atténuer dans la liste** — ils restent en place, l'œil les saute simplement plus
  facilement. <!-- claim:feat-junk-bulk-2 -->
- **Les sortir de la boîte de réception** — retirés de la liste, mais toujours accessibles
  par la recherche et dans les dossiers de votre fournisseur. <!-- claim:feat-junk-bulk-3 -->

<!-- claim:feat-junk-bulk-4 -->
Aucune des deux options ne déplace ni ne supprime quoi que ce soit sur le serveur ; seul un
**Confirmer**, un **Bloquer l'expéditeur** ou un **Pas indésirable** explicite sur un message
des **Indésirables** le fait — ce dernier remet la conversation dans la boîte de réception. Un avertissement d'usurpation d'identité /
hameçonnage est proposé en option, désactivé par défaut.

## Se désabonner et bloquer un expéditeur {#unsubscribe-and-block-sender}

<!-- claim:feat-unsubscribe-1 -->
Une newsletter ou un message de liste de diffusion qui indique comment se désinscrire affiche **Se
désabonner** à côté de son expéditeur. Avant tout envoi, une confirmation explique exactement ce
qui va se passer : une requête envoyée directement au serveur de l'expéditeur (et non via votre
fournisseur de messagerie), un e-mail de désinscription envoyé depuis votre compte, ou la page de
l'expéditeur ouverte dans votre navigateur.

<!-- claim:feat-block-sender-1 -->
**Bloquer l'expéditeur**, dans le menu ⋮ d'une conversation, envoie dans les spams le nouveau
courrier de cet expéditeur sur ce compte et le signale comme spam à votre fournisseur de
messagerie. **Déplacer aussi ses messages existants dans les spams** range ce qui est déjà là, et
la conversation indique que l'expéditeur est bloqué, avec **Débloquer** à portée de main.

<!-- claim:feat-block-sender-2 -->
**Paramètres → Indésirables → Expéditeurs bloqués** liste toutes les personnes bloquées, avec
**Débloquer**, qui peut aussi remettre leurs messages des spams dans la boîte de réception.
**Masquer des filtres intelligents**, dans le menu ⋮, est autre chose : cela retire seulement
l'expéditeur des filtres intelligents de la barre latérale.

## Notifications de nouveau courrier {#new-mail-notifications}

<!-- claim:feat-notifications-1 -->
EmailOps affiche une notification de bureau quand du nouveau courrier arrive dans votre boîte de
réception, et quand une conversation en attente revient — jamais pour la première synchronisation
d'un compte, le courrier ancien, le courrier déjà lu, les indésirables ou les expéditeurs bloqués.
Plus de trois nouveaux messages à la fois sont regroupés en un seul résumé. Cliquer sur une
notification met EmailOps au premier plan ; cela n'ouvre pas le message.

<!-- claim:feat-notifications-2 -->
**Paramètres → Notifications** contient l'interrupteur principal, un interrupteur par compte, le
**Contenu de la notification** (expéditeur et objet, ou **Masquer le contenu**, qui n'affiche que
le compte) et **Seulement quand EmailOps n'est pas au premier plan** ; tout est activé par défaut,
avec l'expéditeur et l'objet affichés. Le texte du message n'est jamais affiché et, tant que
l'application est verrouillée par le mot de passe principal, l'expéditeur et l'objet non plus.

## Contrôles de confidentialité et de sécurité {#privacy-and-security-controls}

<!-- claim:feat-privacy-security-1 -->
Un mot de passe principal verrouille l'application au démarrage, les images distantes et les
pixels de suivi sont bloqués jusqu'à autorisation, et les identifiants résident dans le
trousseau du système. Tout est détaillé dans
[Confidentialité et sécurité](../privacy-security/).

## Interface {#interface}

<!-- claim:feat-interface-1 -->
Boîte en vue divisée ou pleine largeur, et une interface disponible en français, anglais,
espagnol et allemand. La langue de sortie de l'IA se règle séparément : vous pouvez lire
l'interface dans une langue et faire rédiger les réponses dans une autre.

<!-- claim:feat-interface-2 -->
EmailOps propose un thème clair et un thème sombre. **Paramètres → Apparence → Thème** offre
**Clair**, **Sombre** et **Système**, qui suit l'apparence du système d'exploitation ; le choix
est conservé d'un redémarrage à l'autre. Avec le thème sombre, les e-mails s'affichent eux aussi en
sombre dès que c'est possible : un e-mail qui apporte son propre design sombre l'utilise, un e-mail
simple ou au design clair est adapté, et un design déjà sombre est laissé tel quel.

<!-- claim:feat-interface-3 -->
**Paramètres → À propos** affiche la version que vous utilisez, avec des liens vers le site web, le
code source sur GitHub et **Soutenir le projet ♥**, qui ouvre la page GitHub Sponsors du projet.

<!-- claim:feat-interface-4 -->
La barre d'outils de rédaction propose un sélecteur de **Police**, un de **Taille de police** et un
de **Couleur du texte**. Tous trois suivent le curseur : ils montrent la police, la taille et la
couleur du texte où vous vous trouvez. Le texte collé depuis une autre application ne garde ses
couleurs que là où elles se lisent : une couleur trop pâle sur son fond, comme le texte blanc d'une
application en mode sombre, est retirée, pour que le message n'arrive pas avec du texte invisible.
Le bouton **Effacer la mise en forme**, juste après, ramène le texte sélectionné à du texte normal :
il retire gras, italique, polices, tailles et couleurs et transforme un titre en paragraphe normal,
mais garde les liens, les listes et les tableaux.

<!-- claim:feat-interface-5 -->
**Paramètres → Apparence → Densité de la barre latérale** offre **Compacte**, le réglage par
défaut, qui réduit la hauteur des lignes de la liste de la barre latérale pour en afficher davantage
sans défiler, et **Confortable**, plus aérée. Le texte, les icônes ainsi que le haut et le bas de la
barre sont les mêmes dans les deux cas, et le choix est conservé d'un redémarrage à l'autre.

<!-- claim:feat-interface-6 -->
La barre latérale des dossiers et, en vue divisée, la liste des e-mails peuvent être élargies ou
rétrécies : faites glisser la ligne de leur bord droit, ou atteignez-la avec Tab et utilisez les
flèches (Maj pour des pas plus grands), Début et Fin. Double-cliquez sur la ligne pour revenir à la
largeur par défaut. Une colonne cesse de s'élargir avant que le volet de lecture ne descende
sous 320 px, et les largeurs sont conservées d'un redémarrage à l'autre. Si le panneau de droite
(chat ou calendrier) s'ouvre ou si la fenêtre rétrécit, une colonne élargie rend cette largeur, la
liste d'abord, jusqu'à sa largeur par défaut, et la retrouve quand la place revient.

<!-- claim:feat-interface-7 -->
En disposition pleine largeur, la liste des e-mails a des en-têtes de colonne. Cliquez sur
**Expéditeur**, **Objet**, **Date** ou **Taille** pour trier toute la liste selon cette colonne,
toutes ses pages comprises, et cliquez à nouveau pour inverser l'ordre. La liste commence par les
plus récents et revient à cet ordre en vue divisée ; une recherche, un filtre intelligent et **En
attente** gardent leur propre ordre, et dans **Envoyés** la première colonne affiche les
destinataires et ne trie pas. Le bouton **Colonnes**, au bout de l'en-tête, affiche ou masque
**Taille** et **Avec pièce jointe**, et le choix est conservé d'un redémarrage à l'autre. La taille
est celle du message entier, pièces jointes comprises, telle que Gmail ou le serveur IMAP l'indique
à la synchronisation du message. Outlook ne l'indique pas, et le courrier synchronisé avant
l'existence de la colonne ne l'a pas non plus : les deux affichent un tiret.

<!-- claim:feat-interface-8 -->
**Paramètres → Apparence → Affichage des conversations** choisit comment une conversation ouverte
s'affiche. **Ouvrir le dernier message, déplier les autres un par un**, le réglage par défaut, ouvre
le message le plus récent et affiche fermés les précédents, qui s'ouvrent d'un clic. **Votre dernier
message d’abord, tous les messages ouverts** place en haut le dernier message que vous avez envoyé
depuis ce compte et en dessous le reste de la conversation, du plus ancien au plus récent, avec tous
les messages ouverts et chacun une seule fois ; une conversation dans laquelle vous n'avez pas écrit
l'indique et affiche tous les messages du plus ancien au plus récent. Ouvrir une autre conversation
la fait commencer en haut, sauf si elle a été ouverte sur un message précis depuis un lien, qui est
alors affiché comme dans la vue par défaut. L'affichage ne change que ce qui est à l'écran : **Répondre**, **Répondre
à tous** et **Transférer** répondent, citent et joignent les mêmes messages dans les deux cas, et le
choix est conservé d'un redémarrage à l'autre.

## Raccourcis clavier {#keyboard-shortcuts}

<!-- claim:feat-shortcuts-1 -->
Appuyez sur `?` n'importe où en dehors d'un champ de texte pour voir tous les raccourcis. Ils
reprennent ceux de Gmail :

| Touches | Action |
|---|---|
| `j` / `k` | conversation suivante / précédente |
| `Enter` ou `o`, `u` | ouvrir la conversation, revenir à la liste |
| `x` | sélectionner ou désélectionner la conversation |
| `e`, `#`, `s`, `b` | archiver, supprimer, ajouter aux favoris, mettre en attente |
| `Shift+U` / `Shift+I` | marquer comme non lu / lu |
| `c`, `r`, `a`, `f` | nouveau message, répondre, répondre à tous, transférer |
| `g` puis `i`, `s`, `b`, `a`, `l` | aller à Boîte de réception, Favoris, En attente, Archives, Programmés |
| `/` | rechercher |

<!-- claim:feat-shortcuts-2 -->
Les raccourcis sont suspendus pendant la saisie et tant qu'une boîte de dialogue ou un menu est
ouvert. Les boutons qu'ils remplacent indiquent leur touche dans l'info-bulle, comme dans
« Archiver (E) ». **Paramètres → Apparence → Raccourcis clavier** les désactive, et **Afficher la
liste** ouvre le même récapitulatif que `?`.
