### New

- **Chat search filters by priority**, so questions like "urgent promotions" find the right mail.
- **Signed downloads**: Windows installers are code-signed and Linux packages come with signed checksums.
- The CUDA build on Windows tells you when it cannot find CUDA, instead of silently running AI on the CPU.
- **Smart Filters** write their query into the search box: click to search, right-click to add to the current search, and **Show N more** in long groups.

### Privacy and security

- Ollama servers on another machine must use HTTPS.
- Extra hardening for the Windows and Linux builds.

### Fixes

- You can reply from a different account than the one that received the email.
- Replying to a note to yourself fills in the recipients.
- The All-accounts view is faster.
- Adding a Gmail category downloads the mail already in it.
- Long notifications wrap instead of being cut off.
- Chat summaries read the whole thread, chat links open the email they point at, and a new chat puts the cursor in the input.
- Asking the chat for urgent mail only lists urgent mail.
