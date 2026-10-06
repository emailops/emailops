### New

- **Chat skills (experimental).** Save a procedure the chat should follow for one kind of request and turn it on in **Settings → AI Skills**. Start a message with `/name` to use one.
- **Suggested attachment rules.** EmailOps spots recurring attachments, such as monthly invoices, and offers rules to collect them.
- **`/clear` in the chat** starts a new conversation.

### Privacy and security

- OpenRouter never routes your mail to providers that train on it.
- Gmail sign-in uses PKCE, and removing a Gmail account revokes EmailOps' access at Google.
- The main password is throttled after five wrong attempts.

### Fixes

- Gmail sign-in no longer times out during Google's consent screens.
- AI drafts render bold text, and date windows in chat include their last day.
