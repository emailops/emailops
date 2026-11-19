//! Pure planner for the provider draft pull pass.
//!
//! The pull pass used to download every draft in full on every sync tick, which
//! turned a 60-second poll into N+1 provider calls forever. Providers that hand
//! back a per-draft change token in their cheap "list" response let us skip the
//! full read for drafts that have not been touched since the last pull; this
//! module decides which ones those are, with no I/O so it can be tested
//! exhaustively.
//!
//! IMAP has no draft API: its listing is a Drafts folder of immutable
//! messages, turned into the same [`ListedDraft`] shape by
//! [`plan_imap_draft_listing`], and its Drafts folder is only ever read
//! ([`plan_read_only_draft_sync`]).

use std::collections::HashMap;

/// Safety valve on draft-listing pagination. At 100 drafts per page this is
/// 5 000 drafts — far past any real Drafts folder — so reaching it means the
/// provider is handing back a stuck page token rather than making progress.
/// Adapters must return an error at the cap instead of truncating: a partial
/// listing drives `prune_provider_drafts` into deleting drafts it never saw.
pub const MAX_DRAFT_PAGES: usize = 50;

/// A draft as it appears in a provider's cheap list response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedDraft {
    pub provider_draft_id: String,
    /// Token that changes whenever the draft's content changes upstream. Gmail
    /// mints a brand-new message id every time a draft is saved, so its
    /// `draft.message.id` doubles as an ETag. `None` for providers that don't
    /// report one — those drafts are always re-fetched.
    pub change_token: Option<String>,
}

/// What the pull pass should do with the drafts a provider just listed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DraftFetchPlan {
    /// Draft ids whose full content must be downloaded.
    pub to_fetch: Vec<String>,
    /// Every draft id present upstream, changed or not. This is the prune
    /// keep-list: anything stored locally but missing here was sent or deleted
    /// elsewhere.
    pub present_ids: Vec<String>,
}

/// The outcome of one provider draft pull.
///
/// Kept separate from `Vec<ProviderDraft>` because "what changed" and "what
/// exists upstream" are different sets once unchanged drafts stop being read:
/// the prune pass needs the full set, the upsert pass only the changed one.
#[derive(Debug, Clone, Default)]
pub struct ProviderDraftPull {
    /// Drafts whose content was actually downloaded this pass.
    pub changed: Vec<crate::models::ProviderDraft>,
    /// Every draft id present upstream, changed or not — the prune keep-list.
    pub present_ids: Vec<String>,
}

/// Decide which listed drafts need a full content read.
///
/// `known` maps provider draft id → the change token stored the last time that
/// draft's content was pulled.
pub fn plan_draft_fetches(listed: &[ListedDraft], known: &HashMap<String, String>) -> DraftFetchPlan {
    let mut to_fetch = Vec::new();
    let mut present_ids = Vec::with_capacity(listed.len());
    for draft in listed {
        present_ids.push(draft.provider_draft_id.clone());
        // Without a token we cannot prove the draft is untouched, so re-read it.
        let unchanged = draft
            .change_token
            .as_ref()
            .is_some_and(|token| known.get(&draft.provider_draft_id) == Some(token));
        if !unchanged {
            to_fetch.push(draft.provider_draft_id.clone());
        }
    }
    DraftFetchPlan { to_fetch, present_ids }
}

/// One message of an IMAP Drafts folder, as the cheap listing reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImapDraftEntry {
    pub uid: u32,
    /// The message's `Message-ID` header; `None` when it carries none.
    pub message_id: Option<String>,
}

/// Turn an IMAP Drafts folder listing into the [`ListedDraft`]s the pull pass
/// plans with, each paired with the UID its content is fetched by.
///
/// IMAP never edits a message. Another client saves a draft by appending a new
/// message and removing the old one, so the UID changes on every save, and a
/// UIDVALIDITY change renumbers the whole folder. A draft's id is therefore its
/// `Message-ID`, which survives both, and its change token is
/// `<UIDVALIDITY>:<UID>`, which changes with every save and every renumbering:
/// a renumbered folder is read again and lands on the rows it already has
/// instead of duplicating them. A draft without a `Message-ID` is keyed by
/// `uid:<UIDVALIDITY>:<UID>`. When several messages carry one `Message-ID` (a
/// client that appended the new copy and has not removed the old one yet), the
/// newest UID is the draft. Sorted by UID.
pub fn plan_imap_draft_listing(uid_validity: u32, entries: &[ImapDraftEntry]) -> Vec<(u32, ListedDraft)> {
    let mut newest: HashMap<String, u32> = HashMap::new();
    for entry in entries {
        let id = match entry.message_id.as_deref().map(str::trim).filter(|id| !id.is_empty()) {
            Some(message_id) => message_id.to_string(),
            None => format!("uid:{uid_validity}:{}", entry.uid),
        };
        let uid = newest.entry(id).or_insert(entry.uid);
        *uid = (*uid).max(entry.uid);
    }
    let mut listed: Vec<(u32, ListedDraft)> = newest
        .into_iter()
        .map(|(provider_draft_id, uid)| {
            (
                uid,
                ListedDraft {
                    provider_draft_id,
                    change_token: Some(format!("{uid_validity}:{uid}")),
                },
            )
        })
        .collect();
    listed.sort_by_key(|(uid, _)| *uid);
    listed
}

/// What the sync knows about one local draft row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalDraftState {
    /// The row carries a provider draft id (it was pushed or pulled before).
    pub linked: bool,
    /// The row holds user edits that have not reached the provider.
    pub dirty: bool,
}

/// Where the provider copy of a draft stands in the listing just fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamDraftState {
    /// Not in the provider's Drafts folder: never pushed, or sent / deleted
    /// from another device.
    Absent,
    /// Listed, and its change token matches the one stored at the last pull.
    Unchanged,
    /// Listed with content the local row has not seen.
    Changed,
}

/// What the sync does with one draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftSyncAction {
    /// Send the local content to the provider (create when unlinked, update
    /// otherwise) and mark the row clean.
    Push,
    /// Write the provider's content over the local row, or insert it.
    Pull,
    /// The linked provider draft is gone but the local row has unpushed edits:
    /// create it again upstream and re-link, dropping the stale id.
    Recreate,
    /// Delete the local row: it was sent or deleted elsewhere and holds nothing
    /// the provider did not already have.
    Prune,
    /// Leave both sides alone.
    Keep,
}

/// Decide what to do with one draft, given its local row (`None` when the
/// provider lists a draft this device has never stored) and its upstream state.
///
/// The rule the whole table follows: **a dirty local draft is never discarded
/// or overwritten**. It wins a conflict and is pushed; if its provider copy
/// vanished it is created again. Only clean rows are pruned or replaced.
pub fn plan_draft_sync(local: Option<LocalDraftState>, upstream: UpstreamDraftState) -> DraftSyncAction {
    use DraftSyncAction::{Keep, Prune, Pull, Push, Recreate};
    use UpstreamDraftState::{Absent, Changed, Unchanged};
    let Some(local) = local else {
        return if upstream == Changed { Pull } else { Keep };
    };
    match (local.linked, local.dirty, upstream) {
        // A local-only row has no provider copy; the listing says nothing about it.
        (false, false, _) => Keep,
        (false, true, _) => Push,
        (true, false, Absent) => Prune,
        (true, false, Unchanged) => Keep,
        (true, false, Changed) => Pull,
        (true, true, Absent) => Recreate,
        (true, true, Unchanged | Changed) => Push,
    }
}

/// [`plan_draft_sync`] for a provider whose Drafts folder this app only reads
/// (IMAP). Nothing is pushed or re-created upstream: a draft the two-way table
/// would send there stays as it is here, so an edit made in this app is still
/// never overwritten or pruned, and it stays on this device.
pub fn plan_read_only_draft_sync(local: Option<LocalDraftState>, upstream: UpstreamDraftState) -> DraftSyncAction {
    match plan_draft_sync(local, upstream) {
        DraftSyncAction::Push | DraftSyncAction::Recreate => DraftSyncAction::Keep,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(linked: bool, dirty: bool) -> Option<LocalDraftState> {
        Some(LocalDraftState { linked, dirty })
    }

    #[test]
    fn plan_draft_sync_covers_every_local_and_upstream_combination() {
        use DraftSyncAction::{Keep, Prune, Pull, Push, Recreate};
        use UpstreamDraftState::{Absent, Changed, Unchanged};
        let table = [
            // A draft this device has never stored.
            (None, Absent, Keep, "nothing on either side"),
            (None, Unchanged, Keep, "no local row to refresh and no content was read"),
            (None, Changed, Pull, "a draft written elsewhere is imported"),
            // Local-only rows have no provider copy, whatever the listing says.
            (
                local(false, false),
                Absent,
                Keep,
                "a clean local-only draft stays local",
            ),
            (
                local(false, false),
                Unchanged,
                Keep,
                "a clean local-only draft stays local",
            ),
            (
                local(false, false),
                Changed,
                Keep,
                "a clean local-only draft stays local",
            ),
            (
                local(false, true),
                Absent,
                Push,
                "an offline save is pushed on the next sync",
            ),
            (
                local(false, true),
                Unchanged,
                Push,
                "an offline save is pushed on the next sync",
            ),
            (
                local(false, true),
                Changed,
                Push,
                "an offline save is pushed on the next sync",
            ),
            // Linked and in step with the last push/pull.
            (
                local(true, false),
                Absent,
                Prune,
                "sent or deleted elsewhere, nothing local to lose",
            ),
            (local(true, false), Unchanged, Keep, "steady state"),
            (
                local(true, false),
                Changed,
                Pull,
                "edited elsewhere, no local edits to protect",
            ),
            // Linked with unpushed local edits: never discarded.
            (
                local(true, true),
                Absent,
                Recreate,
                "unpushed edits survive an upstream delete",
            ),
            (local(true, true), Unchanged, Push, "local edits reach the provider"),
            (
                local(true, true),
                Changed,
                Push,
                "both sides edited: the local draft wins",
            ),
        ];
        for (local_state, upstream, expected, why) in table {
            assert_eq!(
                plan_draft_sync(local_state, upstream),
                expected,
                "{local_state:?} / {upstream:?}: {why}"
            );
        }
    }

    #[test]
    fn a_dirty_draft_is_never_pruned_or_overwritten() {
        // The invariant behind the table, stated on its own so a future row
        // cannot quietly break it.
        for linked in [false, true] {
            for upstream in [
                UpstreamDraftState::Absent,
                UpstreamDraftState::Unchanged,
                UpstreamDraftState::Changed,
            ] {
                let action = plan_draft_sync(local(linked, true), upstream);
                assert!(
                    matches!(action, DraftSyncAction::Push | DraftSyncAction::Recreate),
                    "dirty draft (linked={linked}, {upstream:?}) planned {action:?}"
                );
            }
        }
    }

    #[test]
    fn a_read_only_provider_is_never_written_to() {
        // IMAP drafts are only read: whatever the two-way table would push or
        // re-create upstream stays as it is here instead.
        for local_state in [
            None,
            local(false, false),
            local(false, true),
            local(true, false),
            local(true, true),
        ] {
            for upstream in [
                UpstreamDraftState::Absent,
                UpstreamDraftState::Unchanged,
                UpstreamDraftState::Changed,
            ] {
                let expected = match plan_draft_sync(local_state, upstream) {
                    DraftSyncAction::Push | DraftSyncAction::Recreate => DraftSyncAction::Keep,
                    other => other,
                };
                assert_eq!(
                    plan_read_only_draft_sync(local_state, upstream),
                    expected,
                    "{local_state:?} / {upstream:?}"
                );
            }
        }
    }

    #[test]
    fn a_read_only_provider_still_imports_refreshes_and_prunes_clean_drafts() {
        use DraftSyncAction::{Keep, Prune, Pull};
        use UpstreamDraftState::{Absent, Changed, Unchanged};
        let table = [
            (None, Changed, Pull, "a draft saved by another client is imported"),
            (
                local(true, false),
                Changed,
                Pull,
                "re-saved elsewhere: the local copy follows",
            ),
            (
                local(true, false),
                Absent,
                Prune,
                "deleted or sent elsewhere: the local copy goes",
            ),
            (local(true, false), Unchanged, Keep, "steady state"),
            (
                local(true, true),
                Changed,
                Keep,
                "an edit made here is never overwritten",
            ),
            (
                local(true, true),
                Absent,
                Keep,
                "an edit made here outlives the server copy",
            ),
            (local(false, true), Absent, Keep, "a draft written here stays here"),
        ];
        for (local_state, upstream, expected, why) in table {
            assert_eq!(
                plan_read_only_draft_sync(local_state, upstream),
                expected,
                "{local_state:?} / {upstream:?}: {why}"
            );
        }
    }

    fn entry(uid: u32, message_id: Option<&str>) -> ImapDraftEntry {
        ImapDraftEntry {
            uid,
            message_id: message_id.map(String::from),
        }
    }

    #[test]
    fn an_imap_draft_is_keyed_by_its_message_id_and_versioned_by_its_uid() {
        let listed = plan_imap_draft_listing(7, &[entry(3, Some("<a@example.test>"))]);
        assert_eq!(listed, vec![(3, listed_draft("<a@example.test>", "7:3"))]);
    }

    #[test]
    fn a_draft_saved_again_by_another_client_keeps_its_id_with_a_new_token() {
        // IMAP never edits a message: a save appends a new one and removes the
        // old, so only the UID tells the two versions apart.
        let before = plan_imap_draft_listing(7, &[entry(3, Some("<a@example.test>"))]);
        let after = plan_imap_draft_listing(7, &[entry(4, Some("<a@example.test>"))]);
        assert_eq!(before[0].1.provider_draft_id, after[0].1.provider_draft_id);
        assert_ne!(before[0].1.change_token, after[0].1.change_token);
    }

    #[test]
    fn a_renumbered_drafts_folder_is_read_again_under_the_same_ids() {
        // A UIDVALIDITY change hands out new UIDs, and an old UID may now be
        // another draft: every draft is read again, and lands on its own row.
        let before = plan_imap_draft_listing(
            7,
            &[entry(3, Some("<a@example.test>")), entry(4, Some("<b@example.test>"))],
        );
        let known: HashMap<String, String> = before
            .iter()
            .map(|(_, d)| (d.provider_draft_id.clone(), d.change_token.clone().expect("token")))
            .collect();
        let after = plan_imap_draft_listing(
            9,
            &[entry(4, Some("<a@example.test>")), entry(3, Some("<b@example.test>"))],
        );
        let after: Vec<ListedDraft> = after.into_iter().map(|(_, d)| d).collect();

        let mut plan = plan_draft_fetches(&after, &known);
        plan.to_fetch.sort();
        plan.present_ids.sort();
        let mut ids: Vec<String> = known.keys().cloned().collect();
        ids.sort();
        assert_eq!(plan.to_fetch, ids, "every draft is read again");
        assert_eq!(plan.present_ids, ids, "no draft looks new or gone");
    }

    #[test]
    fn an_unchanged_imap_drafts_folder_plans_no_read() {
        let listed = plan_imap_draft_listing(7, &[entry(3, Some("<a@example.test>"))]);
        let listed: Vec<ListedDraft> = listed.into_iter().map(|(_, d)| d).collect();
        let plan = plan_draft_fetches(&listed, &known(&[("<a@example.test>", "7:3")]));
        assert!(plan.to_fetch.is_empty());
        assert_eq!(plan.present_ids, vec!["<a@example.test>".to_string()]);
    }

    #[test]
    fn an_imap_draft_without_a_message_id_is_keyed_by_its_uid() {
        for missing in [None, Some(""), Some("   ")] {
            let listed = plan_imap_draft_listing(7, &[entry(3, missing)]);
            assert_eq!(listed, vec![(3, listed_draft("uid:7:3", "7:3"))], "{missing:?}");
        }
    }

    #[test]
    fn the_newest_copy_of_a_message_id_is_the_draft() {
        // A client that appended the new version and has not removed the old
        // one yet: listing both would show the draft twice.
        let listed = plan_imap_draft_listing(
            7,
            &[
                entry(5, Some("<a@example.test>")),
                entry(2, Some("<b@example.test>")),
                entry(8, Some("<a@example.test>")),
            ],
        );
        assert_eq!(
            listed,
            vec![
                (2, listed_draft("<b@example.test>", "7:2")),
                (8, listed_draft("<a@example.test>", "7:8")),
            ]
        );
    }

    #[test]
    fn a_message_id_is_trimmed_like_the_one_the_mail_sync_stores() {
        let listed = plan_imap_draft_listing(7, &[entry(3, Some("  <a@example.test>\r\n"))]);
        assert_eq!(listed[0].1.provider_draft_id, "<a@example.test>");
    }

    fn listed_draft(id: &str, token: &str) -> ListedDraft {
        listed(id, Some(token))
    }

    fn listed(id: &str, token: Option<&str>) -> ListedDraft {
        ListedDraft {
            provider_draft_id: id.to_string(),
            change_token: token.map(String::from),
        }
    }

    fn known(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn a_draft_we_have_never_seen_is_fetched() {
        let plan = plan_draft_fetches(&[listed("d-1", Some("m-1"))], &HashMap::new());
        assert_eq!(plan.to_fetch, vec!["d-1".to_string()]);
    }

    #[test]
    fn a_draft_whose_change_token_still_matches_is_skipped() {
        // The whole point: steady state must issue zero content reads.
        let plan = plan_draft_fetches(&[listed("d-1", Some("m-1"))], &known(&[("d-1", "m-1")]));
        assert!(plan.to_fetch.is_empty(), "unchanged draft must not be re-read");
    }

    #[test]
    fn a_draft_whose_change_token_moved_is_fetched() {
        let plan = plan_draft_fetches(&[listed("d-1", Some("m-2"))], &known(&[("d-1", "m-1")]));
        assert_eq!(plan.to_fetch, vec!["d-1".to_string()]);
    }

    #[test]
    fn a_draft_without_a_change_token_is_always_fetched() {
        // We cannot prove it is unchanged, so correctness beats the saved call.
        let plan = plan_draft_fetches(&[listed("d-1", None)], &known(&[("d-1", "m-1")]));
        assert_eq!(plan.to_fetch, vec!["d-1".to_string()]);
    }

    #[test]
    fn present_ids_lists_skipped_drafts_too() {
        // Regression guard: if skipped drafts fell out of the keep-list, the
        // prune pass would delete every unchanged draft on the next sync.
        let plan = plan_draft_fetches(
            &[listed("d-1", Some("m-1")), listed("d-2", Some("m-2"))],
            &known(&[("d-1", "m-1")]),
        );
        assert_eq!(plan.to_fetch, vec!["d-2".to_string()]);
        assert_eq!(plan.present_ids, vec!["d-1".to_string(), "d-2".to_string()]);
    }

    #[test]
    fn a_locally_known_draft_that_is_gone_upstream_is_not_kept() {
        let plan = plan_draft_fetches(&[listed("d-1", Some("m-1"))], &known(&[("d-1", "m-1"), ("d-2", "m-2")]));
        assert_eq!(plan.present_ids, vec!["d-1".to_string()], "d-2 must be pruned");
    }

    #[test]
    fn an_empty_draft_folder_plans_nothing() {
        let plan = plan_draft_fetches(&[], &known(&[("d-1", "m-1")]));
        assert!(plan.to_fetch.is_empty());
        assert!(plan.present_ids.is_empty());
    }
}
