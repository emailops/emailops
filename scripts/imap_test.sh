#!/usr/bin/env bash
# Local IMAP/SMTP test servers (Dovecot + Mailpit + Toxiproxy) for the
# end-to-end checks of IMAP accounts.
#
#   scripts/imap_test.sh certs   write a test CA + localhost certificate
#   scripts/imap_test.sh up      start the servers (writes certs first)
#   scripts/imap_test.sh down    stop them and drop their mail
#   scripts/imap_test.sh env     print the env vars the live tests read
#
# Everything generated lives under .imap-test/ (gitignored).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
STATE="$ROOT/.imap-test"
CERTS="$STATE/certs"
COMPOSE=(docker compose -f "$ROOT/scripts/imap_test/compose.yml")
PASSWORD="${IMAP_TEST_PASSWORD:-emailops-test}"

certs() {
  [ -f "$CERTS/server.crt" ] && return 0
  mkdir -p "$CERTS"
  cat > "$CERTS/ca.cnf" <<'CNF'
[req]
distinguished_name = dn
prompt = no
x509_extensions = v3_ca
[dn]
CN = EmailOps IMAP Test CA
[v3_ca]
basicConstraints = critical, CA:true
keyUsage = critical, keyCertSign, cRLSign
subjectKeyIdentifier = hash
CNF
  cat > "$CERTS/server.cnf" <<'CNF'
[req]
distinguished_name = dn
prompt = no
[dn]
CN = localhost
[v3_leaf]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature, keyEncipherment
extendedKeyUsage = serverAuth
subjectAltName = DNS:localhost, IP:127.0.0.1
CNF
  openssl req -x509 -new -nodes -newkey rsa:2048 -days 3650 \
    -keyout "$CERTS/ca.key" -out "$CERTS/ca.crt" -config "$CERTS/ca.cnf" 2>/dev/null
  openssl req -new -nodes -newkey rsa:2048 \
    -keyout "$CERTS/server.key" -out "$CERTS/server.csr" -config "$CERTS/server.cnf" 2>/dev/null
  # macOS rejects TLS server certificates valid for more than 825 days.
  openssl x509 -req -in "$CERTS/server.csr" -CA "$CERTS/ca.crt" -CAkey "$CERTS/ca.key" \
    -CAcreateserial -CAserial "$CERTS/ca.srl" -days 800 -extfile "$CERTS/server.cnf" -extensions v3_leaf \
    -out "$CERTS/server.crt" 2>/dev/null
  chmod 644 "$CERTS/server.key"
  echo "test CA written to $CERTS/ca.crt"
}

wait_for_port() {
  local port=$1
  for _ in $(seq 1 60); do
    if (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then return 0; fi
    sleep 1
  done
  echo "port $port did not open" >&2
  return 1
}

up() {
  certs
  "${COMPOSE[@]}" up -d --wait 2>/dev/null || "${COMPOSE[@]}" up -d
  wait_for_port 31143
  wait_for_port 31587
  wait_for_port 8474
  # Toxiproxy proxy in front of Dovecot IMAP; recreated so no toxic survives.
  curl -fsS -X DELETE http://127.0.0.1:8474/proxies/imap >/dev/null 2>&1 || true
  curl -fsS -X POST http://127.0.0.1:8474/proxies \
    -d '{"name":"imap","listen":"0.0.0.0:21143","upstream":"dovecot:31143"}' >/dev/null
  echo "IMAP test servers up (IMAP 31143, submission 31587, proxy 21143, mailpit 8025)"
}

down() {
  "${COMPOSE[@]}" down -v
}

env_vars() {
  cat <<ENV
export EMAILOPS_IMAP_TEST_CA="$CERTS/ca.crt"
export EMAILOPS_IMAP_LIVE_HOST=127.0.0.1
export EMAILOPS_IMAP_LIVE_PORT=31143
export EMAILOPS_IMAP_LIVE_SMTP_PORT=31587
export EMAILOPS_IMAP_LIVE_PASSWORD="$PASSWORD"
export EMAILOPS_IMAP_LIVE_PROXY_PORT=21143
export EMAILOPS_IMAP_LIVE_TOXIPROXY=http://127.0.0.1:8474
export EMAILOPS_IMAP_LIVE_MAILPIT=http://127.0.0.1:8025
ENV
}

case "${1:-}" in
  certs) certs ;;
  up) up ;;
  down) down ;;
  env) env_vars ;;
  *) echo "usage: $0 certs|up|down|env" >&2; exit 2 ;;
esac
