#!/usr/bin/env bash
# Authenticode-sign every Windows artifact under a directory with the
# project's Certum SimplySign cloud certificate, then verify each signature.
#
#   scripts/sign_windows.sh <dir>
#
# Runs on Linux, in the release workflow's signing jobs, which are the only
# jobs that can read the Certum credentials (GitHub environment `signing`,
# owner approval required). Build and bundle jobs never see them: a job that
# runs the project's npm and cargo dependencies must not hold a secret that
# can sign code in the project's name. See docs/DECISIONS.md ("Windows
# artifacts are signed in CI by jobs that never build").
#
#   *.exe, *.dll  ssign, one cloud login for the whole batch
#   *.msi         osslsigncode through ssign's PKCS#11 module, which reuses the
#                 session ssign cached, so the single-use OTP is spent once
#
# Every file is then checked with `osslsigncode verify` against the system
# trust store; one failure fails the run.
#
# Environment:
#   CERTUM_EMAIL, CERTUM_OTP   account e-mail and TOTP seed (required)
#   SSIGN_PKCS11_MODULE        libssign_pkcs11.so (required when an .msi is present)
#   CERTUM_INTERMEDIATE_PEM    Certum Code Signing 2021 CA, added to MSI
#                              signatures (required when an .msi is present)
#   SSIGN, OSSLSIGNCODE        tool paths (default: from PATH)

set -euo pipefail

DIR="${1:-}"
SSIGN="${SSIGN:-ssign}"
OSSLSIGNCODE="${OSSLSIGNCODE:-osslsigncode}"
TIMESTAMP_URL="http://time.certum.pl/"
SIG_NAME="EmailOps"
SIG_URL="https://github.com/emailops/emailops"

die() { echo "ERROR: $*" >&2; exit 1; }

[ -n "$DIR" ] || die "usage: $0 <dir>"
[ -d "$DIR" ] || die "not a directory: $DIR"
[ -n "${CERTUM_EMAIL:-}" ] || die "CERTUM_EMAIL is not set"
[ -n "${CERTUM_OTP:-}" ] || die "CERTUM_OTP is not set"

PE_FILES=()
while IFS= read -r -d '' f; do PE_FILES+=("$f"); done \
  < <(find "$DIR" -type f \( -iname '*.exe' -o -iname '*.dll' \) -print0 | sort -z)
MSI_FILES=()
while IFS= read -r -d '' f; do MSI_FILES+=("$f"); done \
  < <(find "$DIR" -type f -iname '*.msi' -print0 | sort -z)

[ $(( ${#PE_FILES[@]} + ${#MSI_FILES[@]} )) -gt 0 ] || die "no .exe, .dll or .msi under $DIR"

if [ ${#MSI_FILES[@]} -gt 0 ]; then
  [ -f "${SSIGN_PKCS11_MODULE:-}" ] || die "SSIGN_PKCS11_MODULE must point at libssign_pkcs11.so to sign an .msi"
  [ -f "${CERTUM_INTERMEDIATE_PEM:-}" ] || die "CERTUM_INTERMEDIATE_PEM must point at the Certum intermediate to sign an .msi"
fi

# PE first: this login is the one that caches the session the PKCS#11
# module picks up for the MSIs below.
if [ ${#PE_FILES[@]} -gt 0 ]; then
  echo "[sign-windows] signing ${#PE_FILES[@]} PE file(s) with ssign"
  "$SSIGN" -n "$SIG_NAME" -u "$SIG_URL" "${PE_FILES[@]}"
fi

# The single object ssign-pkcs11 exposes, selected by type.
CERT_URI="pkcs11:object=Certum%20SimplySign%20%28ssign%29;type=cert"
KEY_URI="pkcs11:object=Certum%20SimplySign%20%28ssign%29;type=private"
for msi in "${MSI_FILES[@]}"; do
  echo "[sign-windows] signing $(basename "$msi") with osslsigncode + ssign-pkcs11"
  "$OSSLSIGNCODE" sign -pkcs11module "$SSIGN_PKCS11_MODULE" \
    -pkcs11cert "$CERT_URI" -key "$KEY_URI" \
    -ac "$CERTUM_INTERMEDIATE_PEM" -h sha256 -t "$TIMESTAMP_URL" \
    -n "$SIG_NAME" -i "$SIG_URL" \
    -in "$msi" -out "$msi.signed"
  mv "$msi.signed" "$msi"
done

FAILED=()
for f in "${PE_FILES[@]}" "${MSI_FILES[@]}"; do
  if "$OSSLSIGNCODE" verify -in "$f" > "$f.verify.log" 2>&1; then
    echo "[sign-windows] verified $f"
  else
    echo "[sign-windows] verification FAILED for $f:" >&2
    sed 's/^/    /' "$f.verify.log" >&2
    FAILED+=("$f")
  fi
  rm -f "$f.verify.log"
done

[ ${#FAILED[@]} -eq 0 ] || die "signature verification failed for: ${FAILED[*]}"
echo "[sign-windows] signed and verified $(( ${#PE_FILES[@]} + ${#MSI_FILES[@]} )) file(s)"
