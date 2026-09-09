#!/usr/bin/env bash
# Smoke test for the feature/sync-speedup branch.
#
# Each check maps to one claim the branch makes. Run from the repo root:
#   bash scripts/smoke_sync_speedup.sh
set -uo pipefail

cd "$(dirname "$0")/.."
OUT=$(mktemp)
trap 'rm -f "$OUT"' EXIT

echo "Running the suite (lib + integration, no-default-features)…"
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features > "$OUT" 2>&1
SUITE_STATUS=$?

fail=0
check() { # $1 = test name, $2 = human label
  if grep -qE "^test .*${1} \.\.\. ok" "$OUT"; then
    printf '  PASS  %s\n' "$2"
  else
    printf '  FAIL  %s\n' "$2"; fail=1
  fi
}

echo
echo "IMAP: one connection per chunk instead of one per message"
check "batch_fetch_returns_every_body_from_a_single_command"        "whole chunk fetched in one UID FETCH"
check "batch_fetch_retries_past_an_interleaved_response"            "interleaved untagged response retried"
check "batch_fetch_omits_a_uid_the_server_did_not_return"           "a dropped UID costs only its own message"
check "batch_fetch_of_nothing_issues_no_command"                    "empty set issues no command"
check "batch_fetch_propagates_a_tagged_no_without_retrying"         "tagged NO propagates"
check "a_batch_from_one_folder_becomes_a_single_select_and_fetch"   "one folder -> one SELECT + one FETCH"
check "a_mixed_batch_is_grouped_per_folder_keeping_each_messages_slot" "mixed folders keep positional slots"
check "a_malformed_uid_fails_only_its_own_message"                  "bad ID isolated from the chunk"
check "an_empty_batch_plans_no_work"                                "empty chunk plans nothing"

echo
echo "Outlook: Graph \$batch instead of 20 sequential GETs"
check "batch_payload_asks_for_every_message_in_one_request"         "one request carries the whole chunk"
check "batch_payload_ids_the_sub_requests_by_slot"                  "sub-requests identified by slot"
check "batch_payload_percent_encodes_the_message_id"                "message IDs percent-encoded"
check "batch_responses_are_matched_by_id_not_by_position"           "out-of-order responses de-shuffled"
check "a_throttled_sub_response_is_reported_with_its_status"        "per-message 429 surfaced for retry"
check "a_malformed_batch_envelope_yields_no_sub_responses"          "malformed envelope never panics"
check "outlook_batch_get_messages_against_cassette_mock"            "end-to-end against the wiremock cassette"

echo
echo "Pacing: quota-derived instead of a flat 2s per 20 messages"
check "gmail_pacing_keeps_a_backfill_inside_the_documented_quota"   "Gmail stays inside 250 units/s"
check "every_known_provider_beats_the_old_flat_two_seconds"         "every known provider is faster than before"
check "an_unknown_provider_keeps_the_conservative_pacing"           "unknown provider keeps the safe default"

echo
echo "Incremental display: mail lands while the mailbox is still being listed"
check "a_large_backfill_downloads_while_it_is_still_listing"     "downloading starts after one slice, listing continues"

echo
echo "Inbox filter: a synced category is never hidden by default"
if npx vitest run src/lib/categories.test.ts >/dev/null 2>&1; then
  printf '  PASS  %s\n' "default filter covers every category the account may sync"
else
  printf '  FAIL  %s\n' "default filter covers every category the account may sync"; fail=1
fi

echo
echo "Guards: the old per-message paths still work"
check "outlook_client_list_messages_against_cassette_mock"          "Outlook list_messages unchanged"
check "search_query_is_all_when_unbounded"                          "IMAP search query builder unchanged"

echo
grep -E "^test result" "$OUT"
if [ $SUITE_STATUS -ne 0 ] || [ $fail -ne 0 ]; then
  echo
  echo "SMOKE FAILED — full output:"; cat "$OUT"; exit 1
fi
echo
echo "SMOKE OK"
