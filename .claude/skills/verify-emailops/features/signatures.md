# Signatures

Each account has one HTML signature (Settings → **Signatures**), inserted in new messages and/or
replies and forwards, editable or removable in any message. Gmail accounts can import theirs.

## Sub-features

- `signatures.edit` per-account editor with Save / Discard and the two insertion switches.
- `signatures.insert` the composer opens with the signature (`[data-emailops-signature]`) below the caret; replies put it below the text, forwards above the quoted message.
- `signatures.import` Import from Gmail (Gmail accounts only).
- `signatures.image` **Add image**: PNG, JPEG, GIF or WebP only; wider than 600 px or over 200 KB is redrawn at 600 px; still over 200 KB, SVG or anything else is refused with the reason. The backend re-checks every `data:` image on save (type, base64, ≤ 200 KB, magic bytes, ≤ 1200 px wide).
- `signatures.plain` the text/plain part of a sent message puts `-- ` before a closing signature (not in a forward, where the quoted message follows).

## How to get to it (user POV)

- Gear → **Signatures**; then any Compose / Reply / Forward.

## Driving it with verify.sh

Preconditions: baseline (no signature saved). Confirmed live on 02/10/2026.

- Save → open Settings, the **Signatures** tab, type into `[data-testid=signatures-settings] [contenteditable=true]`, click **Save signature** → *Signature saved*; `select html from account_signatures where account_id='demo-acct-work'` returns it.
- Insert → close Settings, **Compose** → `[contenteditable=true] [data-emailops-signature]` holds the text.
- Image → `[data-testid=signature-image-input]` is hidden; feed it a `File` through a `DataTransfer` and dispatch `change` (WebDriver cannot drive the file dialog). An SVG shows *PNG, JPEG, GIF or WebP*; a 1600 px PNG lands as a 600 px `<img>`, saved as `data:image/png`.
- Clean up → empty the editor and save again (the row stays, with `html = ''`).

## Gotchas

- Saving an empty editor keeps the row with an empty `html`; that is the "no signature" state.
- Import from Gmail calls the provider: not driven on the demo (credential-less).

| Case | Test kind |
|---|---|
| sanitising, size limit, get/save/import, image type/size/width/base64 checks | unit (`services::signatures`) |
| editor, store, composer insertion, reply placement, image planner/executor, `-- ` in text/plain | vitest (`signature`, `signatureStore`, `useComposerSignature`, `SignaturesSettings`, `ReplyCompose.signature`, `signatureImage`, `composeHtml`) |
| save → read back clean | integration (`signature_saved_for_an_account_is_read_back_without_scripts`) |
| command arguments | contract (`src/lib/apiContract/cuentas.api.test.ts`, `accounts.rs`) |
| save, insertion in a new message, refused SVG, downscaled PNG, clean-up | e2e (`Firmas/*`) |
| eval | n/a: no model involved |
