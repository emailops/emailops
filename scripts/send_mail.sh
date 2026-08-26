#!/usr/bin/env bash
# Hand a ready-made RFC 5322 message to an SMTP server via curl.
#
# curl speaks SMTP, so this needs no mail library and no third-party Action —
# one less dependency with access to the repo's secrets. The message (headers,
# MIME parts, base64 bodies) is built by `scripts/repo_metrics.mjs`; this
# script only transports it.
#
# Required environment (GitHub Actions secrets):
#   SMTP_URL   e.g. smtps://smtp.gmail.com:465  (smtps:// = implicit TLS)
#   SMTP_USER  SMTP login
#   SMTP_PASS  SMTP password — for Gmail/iCloud this must be an app password,
#              never the account password
#   MAIL_FROM  envelope sender, usually the same as SMTP_USER
#   MAIL_TO    recipient
#
#   scripts/send_mail.sh <message-file>

set -euo pipefail

[ $# -eq 1 ] || { echo "usage: $0 <message-file>" >&2; exit 2; }
message="$1"

[ -s "$message" ] || { echo "[mail] $message is empty — refusing to send" >&2; exit 1; }

missing=()
for var in SMTP_URL SMTP_USER SMTP_PASS MAIL_FROM MAIL_TO; do
  [ -n "${!var:-}" ] || missing+=("$var")
done
if [ ${#missing[@]} -gt 0 ]; then
  echo "[mail] missing required secrets: ${missing[*]}" >&2
  echo "[mail] set them under Settings → Secrets and variables → Actions" >&2
  exit 1
fi

# Options go in on stdin rather than as arguments so the password never appears
# in the process list, where any other process on the runner could read it.
# --ssl-reqd refuses to fall back to an unencrypted session: credentials must
# never cross the wire in the clear, even if the server offers to.
curl --config - <<EOF
url = "$SMTP_URL"
user = "$SMTP_USER:$SMTP_PASS"
mail-from = "$MAIL_FROM"
mail-rcpt = "$MAIL_TO"
upload-file = "$message"
ssl-reqd
silent
show-error
fail
EOF

echo "[mail] sent to the configured recipient"
