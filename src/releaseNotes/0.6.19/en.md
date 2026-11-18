**Enjoying EmailOps?** Support the project with a [star on GitHub](https://github.com/emailops/emailops): it helps more people find it.

### New

- **Bcc and editable subjects**: every composer has a Bcc field, and the subject of a reply or a forward can be changed before it goes. Replies go to the Reply-To address, such as a mailing list's.
- **Drafts keep their attachments**: a file is saved with the draft as soon as you add it, and forwards are saved as drafts too.
- **Boolean search**: quotes for an exact phrase, `OR`, `-word` or `NOT` to leave a word out, and brackets to group. Plus `cc:` and `has:attachment` to filter.
- **New mail is ready for AI search sooner**: Embeddings are generated newest first, across all your accounts.

### Fixes

- IMAP: a deleted message shows once in Deleted, and on Proton Mail Bridge it is no longer erased for good.
- IMAP: accounts connect even when the first address of their server does not answer.
- IMAP: on older servers, moving a message no longer erases others another app marked for deletion.
- IMAP: a server reply EmailOps cannot read no longer stops the sync.
- AI drafts follow your instructions with any template, and a long brief is no longer cut short without a word.
- Gmail attachments kept with the message open again.
- A draft Gmail refuses is no longer sent again every minute.
