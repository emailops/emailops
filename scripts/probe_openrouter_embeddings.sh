#!/usr/bin/env bash
# Probe every OpenRouter embedding model with the request shape EmailOps sends
# (`data_collection: "deny"`, so providers that train on or store prompts are
# excluded) and report which ones can fill the app's 768-dimension vector
# tables: through the `dimensions` parameter, natively, or not at all. A third
# column says whether the model still answers with zero data retention on.
#
# One short fixed string is embedded per attempt — no mail content. Each
# attempt is a paid call (fractions of a cent in total).
#
# Usage: OPENROUTER_API_KEY=... scripts/probe_openrouter_embeddings.sh [out.tsv]
#        (make probe-openrouter-embeddings)
# Prints a TSV: model, price per 1M tokens, context, via `dimensions`, native,
# with ZDR. Cells hold the vector length returned, or `err:<http status>`.
set -euo pipefail

: "${OPENROUTER_API_KEY:?set OPENROUTER_API_KEY in the environment}"
out="${1:-/dev/stdout}"
base="https://openrouter.ai/api/v1"
text="EmailOps embedding check"

# $1 model, $2 extra JSON merged into the body, $3 provider preferences JSON.
attempt() {
  local body status reply
  body=$(jq -n --arg m "$1" --arg t "$text" --argjson extra "$2" --argjson prov "$3" \
    '{model: $m, input: $t, encoding_format: "float", provider: $prov} + $extra')
  reply=$(mktemp)
  status=$(curl -sS -o "$reply" -w '%{http_code}' --max-time 60 \
    -H "Authorization: Bearer $OPENROUTER_API_KEY" -H 'Content-Type: application/json' \
    -d "$body" "$base/embeddings") || status="000"
  if [[ "$status" == 2* ]] && jq -e '.data[0].embedding | type == "array"' "$reply" >/dev/null 2>&1; then
    jq -r '.data[0].embedding | length' "$reply"
  else
    echo "err:$status"
  fi
  rm -f "$reply"
}

deny='{"data_collection": "deny"}'
zdr='{"data_collection": "deny", "zdr": true}'

models=$(curl -sS --max-time 60 -H "Authorization: Bearer $OPENROUTER_API_KEY" "$base/embeddings/models")

{
  printf 'model\tusd_per_1m_tokens\tcontext\tdimensions_768\tnative\tzdr\n'
  jq -r '.data[] | [.id, ((.pricing.prompt // "0" | tonumber) * 1000000 | tostring), (.context_length // 0 | tostring)] | @tsv' <<<"$models" |
    while IFS=$'\t' read -r id price ctx; do
      with_dims=$(attempt "$id" '{"dimensions": 768}' "$deny")
      native=$(attempt "$id" '{}' "$deny")
      if [[ "$with_dims" == 768 ]]; then
        with_zdr=$(attempt "$id" '{"dimensions": 768}' "$zdr")
      else
        with_zdr=$(attempt "$id" '{}' "$zdr")
      fi
      printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$id" "$price" "$ctx" "$with_dims" "$native" "$with_zdr"
    done
} >"$out"
