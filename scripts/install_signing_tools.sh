#!/usr/bin/env bash
# Install what scripts/sign_windows.sh needs on a Linux CI runner and export
# the tool paths to later steps through $GITHUB_ENV.
#
#   scripts/install_signing_tools.sh [dest-dir]
#
# ssign is built from a pinned, reviewed commit, never `latest`: the signing
# job hands it the Certum TOTP seed, which can sign code in the project's name
# until the SimplySign QR code is re-issued. Before moving SSIGN_REV, read the
# diff of ssign-core/src/{auth,client,session}.rs — that is where the e-mail,
# the seed and the session token travel.

set -euo pipefail

SSIGN_REPO="https://github.com/Le-Syl21/ssign"
SSIGN_REV="5fd4daf22155b19b645aef3dd467f3c4c9e440b3" # ssign 0.1.7, reviewed 02/10/2026
DEST="${1:-${RUNNER_TEMP:?}/ssign}"

sudo apt-get update
# osslsigncode signs the MSIs through ssign's PKCS#11 module, which OpenSSL
# loads via libp11's engine.
sudo apt-get install -y osslsigncode libengine-pkcs11-openssl

git clone --quiet "$SSIGN_REPO" "$DEST"
git -C "$DEST" checkout --quiet "$SSIGN_REV"
if [ "$(git -C "$DEST" rev-parse HEAD)" != "$SSIGN_REV" ]; then
  echo "ERROR: ssign checkout is not at the pinned commit $SSIGN_REV" >&2
  exit 1
fi
cargo build --release --locked --manifest-path "$DEST/Cargo.toml" -p ssign -p ssign-pkcs11

# The Certum Code Signing 2021 CA that issued the certificate; osslsigncode
# needs it to embed the full chain in MSI signatures (ssign embeds it itself).
openssl x509 -inform DER -in "$DEST/ssign-core/src/certs/ccsca2021.der" -out "$DEST/ccsca2021.pem"

{
  echo "SSIGN=$DEST/target/release/ssign"
  echo "SSIGN_PKCS11_MODULE=$DEST/target/release/libssign_pkcs11.so"
  echo "CERTUM_INTERMEDIATE_PEM=$DEST/ccsca2021.pem"
} >> "${GITHUB_ENV:?}"
echo "[install-signing-tools] ssign $SSIGN_REV, $(osslsigncode --version 2>&1 | head -1)"
