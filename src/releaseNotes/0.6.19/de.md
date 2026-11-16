**Gefällt dir EmailOps?** Unterstütze das Projekt mit einem [Stern auf GitHub](https://github.com/emailops/emailops): So finden es mehr Menschen.

### Neu

- **Bcc und bearbeitbare Betreffzeilen**: Jedes Verfassen-Fenster hat ein Bcc-Feld, und der Betreff einer Antwort oder Weiterleitung lässt sich vor dem Senden ändern. Antworten gehen an die Reply-To-Adresse, etwa die einer Mailingliste.
- **Entwürfe behalten ihre Anhänge**: Eine Datei wird mit dem Entwurf gespeichert, sobald du sie hinzufügst, und auch Weiterleitungen werden als Entwürfe gespeichert.
- **Boolesche Suche**: Anführungszeichen für eine genaue Wortfolge, `OR`, `-wort` oder `NOT`, um ein Wort auszuschließen, und Klammern zum Gruppieren. Dazu `cc:` und `has:attachment` zum Filtern.
- **Neue E-Mails sind früher für die KI-Suche bereit**: Embeddings werden von den neuesten zu den ältesten erzeugt, über alle deine Konten.

### Korrekturen

- IMAP: Eine gelöschte Nachricht erscheint nur einmal in Gelöscht, und bei Proton Mail Bridge wird sie nicht mehr endgültig gelöscht.
- IMAP: Konten verbinden sich, auch wenn die erste Adresse ihres Servers nicht antwortet.
- IMAP: Auf älteren Servern löscht das Verschieben einer Nachricht keine anderen mehr, die eine andere App zum Löschen markiert hat.
- IMAP: Eine Serverantwort, die EmailOps nicht lesen kann, hält die Synchronisierung nicht mehr an.
- KI-Entwürfe folgen deinen Anweisungen mit jeder Vorlage, und eine lange Anweisung wird nicht mehr stillschweigend gekürzt.
- Gmail-Anhänge, die mit der Nachricht gespeichert sind, lassen sich wieder öffnen.
- Ein Entwurf, den Gmail ablehnt, wird nicht mehr jede Minute erneut gesendet.
