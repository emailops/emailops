#!/usr/bin/env bash
# Write and GPG-sign the SHA256 checksums of the Linux installers.
#
#   scripts/sign_linux_checksums.sh <dir>
#
# Produces <dir>/EmailOps-linux-SHA256SUMS (one `sha256sum` line per .deb,
# .AppImage and .rpm in <dir>) and its detached, armored signature
# EmailOps-linux-SHA256SUMS.asc. Users check the signature against the
# project's public key, then the files against the sums — the "GPG-signed
# checksums published with the download" route to an integrity-verified
# Linux download.
#
# Runs in the release workflow's linux-sign job, in the `signing` environment,
# the only place RELEASE_GPG_PRIVATE_KEY exists. Before publishing anything it
# checks the signature against the public key committed in the repo, so a
# secret that drifted from the published key fails the release instead of
# shipping signatures nobody can verify.
#
# Environment:
#   RELEASE_GPG_PRIVATE_KEY      armored private key (required)
#   RELEASE_SIGNING_PUBLIC_KEY   published public key
#                                (default: docs/release-signing-key.asc)

set -euo pipefail

DIR="${1:-}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PUBLIC_KEY="${RELEASE_SIGNING_PUBLIC_KEY:-$ROOT/docs/release-signing-key.asc}"
SUMS="EmailOps-linux-SHA256SUMS"

die() { echo "ERROR: $*" >&2; exit 1; }

[ -n "$DIR" ] || die "usage: $0 <dir>"
[ -d "$DIR" ] || die "not a directory: $DIR"
[ -n "${RELEASE_GPG_PRIVATE_KEY:-}" ] || die "RELEASE_GPG_PRIVATE_KEY is not set"
[ -f "$PUBLIC_KEY" ] || die "published public key not found at $PUBLIC_KEY"

FILES=()
while IFS= read -r f; do FILES+=("$f"); done \
  < <(cd "$DIR" && find . -maxdepth 1 -type f \( -name '*.deb' -o -name '*.AppImage' -o -name '*.rpm' \) | sed 's|^\./||' | sort)
[ ${#FILES[@]} -gt 0 ] || die "no .deb, .AppImage or .rpm in $DIR"

if command -v sha256sum >/dev/null 2>&1; then SHA=(sha256sum); else SHA=(shasum -a 256); fi

# Two throwaway keyrings: one holds the private key and signs, the other
# holds only the published public key and verifies.
SIGN_HOME="$(mktemp -d)"
VERIFY_HOME="$(mktemp -d)"
cleanup() {
  GNUPGHOME="$SIGN_HOME" gpgconf --kill gpg-agent 2>/dev/null || true
  GNUPGHOME="$VERIFY_HOME" gpgconf --kill gpg-agent 2>/dev/null || true
  rm -rf "$SIGN_HOME" "$VERIFY_HOME"
}
trap cleanup EXIT
chmod 700 "$SIGN_HOME" "$VERIFY_HOME"

printf '%s\n' "$RELEASE_GPG_PRIVATE_KEY" | GNUPGHOME="$SIGN_HOME" gpg --batch --quiet --import

(cd "$DIR" && "${SHA[@]}" "${FILES[@]}") > "$DIR/$SUMS.tmp"
GNUPGHOME="$SIGN_HOME" gpg --batch --yes --pinentry-mode loopback --passphrase '' \
  --armor --detach-sign --output "$DIR/$SUMS.asc.tmp" "$DIR/$SUMS.tmp"

GNUPGHOME="$VERIFY_HOME" gpg --batch --quiet --import "$PUBLIC_KEY"
if ! GNUPGHOME="$VERIFY_HOME" gpg --batch --verify "$DIR/$SUMS.asc.tmp" "$DIR/$SUMS.tmp" 2>"$DIR/$SUMS.verify.log"; then
  cat "$DIR/$SUMS.verify.log" >&2
  rm -f "$DIR/$SUMS.tmp" "$DIR/$SUMS.asc.tmp" "$DIR/$SUMS.verify.log"
  die "the signature does not verify against the published key $PUBLIC_KEY"
fi
rm -f "$DIR/$SUMS.verify.log"

mv "$DIR/$SUMS.tmp" "$DIR/$SUMS"
mv "$DIR/$SUMS.asc.tmp" "$DIR/$SUMS.asc"
echo "[sign-linux] signed checksums of ${#FILES[@]} file(s):"
sed 's/^/    /' "$DIR/$SUMS"
