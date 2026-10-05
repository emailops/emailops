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

## Boîte de réception unifiée {#unified-inbox}

<!-- claim:feat-unified-inbox-1 -->
La vue **Tous les comptes** fusionne chaque boîte activée en une seule liste, à côté des vues
par compte. Les dossiers IMAP personnalisés sont également synchronisés, et vous pouvez les
créer, les renommer, les supprimer et y déplacer des messages par glisser-déposer depuis
l'application.

## Transférer

<!-- claim:reading-pane-forward -->
**Transférer** se trouve à côté de **Répondre** et **Répondre à tous** dans le volet de
lecture. Le brouillon s'ouvre sans destinataire et contient le message d'origine sous un
en-tête *Message transféré* avec son expéditeur, sa date et ses destinataires, ainsi que les
pièces jointes d'origine (jusqu'à 20 Mo au total). Il part comme un nouveau message et ne
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

<!-- claim:feat-organize-5 -->
Quand la conversation que vous lisez quitte la liste — archivée, supprimée, mise en attente ou
déplacée dans les spams —, la suivante s'ouvre. **Paramètres → Apparence → Après l’archivage ou
la suppression** permet de choisir entre la conversation suivante, la précédente ou le retour à
la liste. Marquer une conversation comme non lue ramène toujours à la liste.

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

## Filtres intelligents

<!-- claim:feat-smart-filters-1 -->
Restreignez la liste par domaine, expéditeur ou étiquette de classification — pratique pour
traiter un client, un projet ou un déluge de newsletters à la fois. Avec l'IA activée, ces
mêmes étiquettes alimentent aussi le [Tableau d'étiquettes](../ai-features/#tag-board), qui
les présente sous forme de grille de blocs.

## Calendrier {#calendar}

<!-- claim:feat-calendar-1 -->
Vues mois, semaine et jour par compte pour Google Agenda et Outlook. Vous recevez des rappels
avant chaque événement, avec un bouton **Rejoindre** en un clic pour les liens Meet, Teams,
Webex et Zoom. La synchronisation du calendrier est active par défaut pour les comptes Gmail
et Outlook et peut être désactivée compte par compte, tout comme le délai de notification,
dans **Paramètres → Calendrier**.

<!-- claim:feat-calendar-2 -->
Tous les agendas d'un compte sont synchronisés, pas seulement le principal — un agenda
qu'un collègue a partagé avec vous apparaît donc ici comme dans Google ou Outlook. Chacun
prend la couleur que lui donne son fournisseur, et la légende au-dessus de la grille masque
ou affiche les agendas un par un ; les mêmes interrupteurs se trouvent dans
**Paramètres → Calendrier**.

## Vue des pièces jointes {#attachments-view}

<!-- claim:feat-attachments-view-1 -->
Un seul endroit pour les pièces jointes qui comptent — factures, contrats, reçus — avec aperçu
et téléchargement, au lieu de fouiller à nouveau les fils de discussion. Ouvrez-la depuis
**Pièces jointes** dans la barre latérale.

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
**Importer** transforme un document Word (`.docx`) ou une feuille de calcul (`.xlsx`, `.xls`, `.ods`)
en EO Docs, une feuille par onglet et les formules sous forme de valeurs ; **Ouvrir dans EO Docs**
fait de même avec une pièce jointe. Dans l'éditeur d'e-mails, **Depuis EO Docs** joint un document,
ce qui le partage avec les destinataires de l'e-mail.

## Recherche

<!-- claim:feat-search-1 -->
Recherche plein texte sur les objets, les corps, les expéditeurs et les pièces jointes. Avec
l'IA activée s'y ajoute la recherche sémantique, qui correspond au sens plutôt qu'aux mots
exacts.

<!-- claim:feat-search-2 -->
Les recherches se précisent avec des opérateurs, seuls ou à côté de texte libre :

| Opérateur | Recherche |
|---|---|
| `from:ana` | adresse ou nom de l'expéditeur |
| `to:ana` | destinataire |
| `subject:facture` | objet |
| `before:2026-09-01` / `after:2026-09-01` | date de réception |
| `id:<id du courriel>` | un courriel précis |
| `tag:newsletter` / `tag:intent=request` | une étiquette du classifieur, éventuellement dans une facette |

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
**Confirmer** ou un **Bloquer l'expéditeur** explicite le fait. Un avertissement d'usurpation d'identité /
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
