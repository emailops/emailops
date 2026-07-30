//! User-facing configuration and status for junk detection.
//!
//! Deliberately small. Three switches reach the user; the calibration constants
//! do not, and that is a design decision rather than an omission — see
//! [`JunkConfig`].

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::models::error::Result;
use crate::services::junk::model::{ModelAxis, NaiveBayes};

/// What the inbox does with a flagged message.
///
/// Never "delete" and never "move on the server": the detector's promise is that
/// a verdict is a local flag, so the strongest option available here still
/// leaves the message exactly where the server put it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlaggedAction {
    /// Badge it and fade the row. Recoverable at a glance.
    Dim,
    /// Keep it out of the inbox list. Still reachable through search and the
    /// server's own folders — nothing is destroyed.
    Hide,
}

impl FlaggedAction {
    fn parse(s: &str) -> Self {
        match s {
            "hide" => FlaggedAction::Hide,
            _ => FlaggedAction::Dim,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            FlaggedAction::Dim => "dim",
            FlaggedAction::Hide => "hide",
        }
    }
}

/// The settings a user may change.
///
/// What is **absent** matters as much as what is present. The class priors
/// (`junk_spam_prior`, `junk_graymail_prior`) and every signal weight and band
/// cutoff stay out of reach on purpose:
///
/// * A prior is the base rate of junk in the inbox. Nobody can know their own —
///   it took measuring 613 messages to establish ~1% on the mailbox this was
///   built against. And the cost of a wrong value is invisible: at a prior of
///   0.25 instead of 0.01 the classifier needs roughly thirty times less
///   evidence before it accuses, so "turn it up to catch more" quietly turns the
///   feature into a false-positive generator with no signal that anything broke.
/// * Weights and cutoffs come off a measured precision/recall curve. A user
///   moving one has no feedback loop to judge the result against.
///
/// A control that degrades the system silently is worse than no control.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JunkConfig {
    /// Off by default. The detector accuses mail of being junk and fades or
    /// hides rows on the strength of that; an install that never opted in does
    /// not get that done to it silently.
    pub enabled: bool,
    /// Off by default: this axis renders an accusation of impersonation and has
    /// far too little ground truth behind it to be trusted with one.
    pub phishing_enabled: bool,
    pub flagged_action: FlaggedAction,
}

pub fn get_config(db: &Arc<Database>) -> JunkConfig {
    let pref = |key: &str| db.get_preference(key).ok().flatten();
    JunkConfig {
        enabled: pref("junk_enabled").map(|v| v == "true").unwrap_or(false),
        phishing_enabled: pref("junk_phishing_enabled").map(|v| v == "true").unwrap_or(false),
        flagged_action: pref("junk_flagged_action")
            .map(|v| FlaggedAction::parse(&v))
            .unwrap_or(FlaggedAction::Dim),
    }
}

pub fn save_config(db: &Arc<Database>, config: &JunkConfig) -> Result<()> {
    db.set_preference("junk_enabled", if config.enabled { "true" } else { "false" })?;
    db.set_preference(
        "junk_phishing_enabled",
        if config.phishing_enabled { "true" } else { "false" },
    )?;
    db.set_preference("junk_flagged_action", config.flagged_action.as_str())?;
    Ok(())
}

/// One trained model's state, for the read-only status block.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JunkModelInfo {
    pub axis: String,
    pub positives: u32,
    pub negatives: u32,
    /// False when the model has too few labels to be allowed to vote. Shown
    /// because "trained" and "in use" are not the same thing, and a user seeing
    /// counts with no effect deserves to know which it is.
    pub in_use: bool,
    pub trained_at: Option<i64>,
}

/// What the detector has actually done, so the feature is not a black box.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JunkStats {
    pub scored: i64,
    pub unscored: i64,
    pub phishing: i64,
    pub spam: i64,
    pub graymail: i64,
    pub marked_junk: i64,
    pub marked_not_junk: i64,
    pub models: Vec<JunkModelInfo>,
}

pub fn get_stats(db: &Arc<Database>, account_id: &str) -> Result<JunkStats> {
    let counts = db.junk_stats_counts(account_id)?;

    let mut models = Vec::new();
    for axis in ModelAxis::ALL {
        if let Some((model, _version)) = db.load_junk_model(account_id, axis)? {
            models.push(JunkModelInfo {
                axis: axis.as_str().to_string(),
                positives: model.n_pos,
                negatives: model.n_neg,
                in_use: NaiveBayes::is_usable(&model),
                trained_at: db.junk_model_trained_at(account_id, axis).ok().flatten(),
            });
        } else {
            models.push(JunkModelInfo {
                axis: axis.as_str().to_string(),
                positives: 0,
                negatives: 0,
                in_use: false,
                trained_at: None,
            });
        }
    }

    Ok(JunkStats {
        scored: counts.scored,
        unscored: counts.unscored,
        phishing: counts.phishing,
        spam: counts.spam,
        graymail: counts.graymail,
        marked_junk: counts.marked_junk,
        marked_not_junk: counts.marked_not_junk,
        models,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Arc<Database> {
        Arc::new(Database::new_for_testing().expect("test db"))
    }

    #[test]
    fn junk_detection_is_off_until_the_user_asks_for_it() {
        // The detector accuses mail of being junk and fades or hides rows on the
        // strength of that. It does not get to start doing so on an install that
        // never opted in — `is_enabled` gates both the sync hook and the
        // backfill, so an unset preference means no scoring at all.
        assert!(!get_config(&test_db()).enabled);
    }

    #[test]
    fn phishing_stays_off_by_default_too() {
        assert!(!get_config(&test_db()).phishing_enabled);
    }

    #[test]
    fn flagged_action_defaults_to_the_recoverable_one() {
        assert_eq!(get_config(&test_db()).flagged_action, FlaggedAction::Dim);
    }

    #[test]
    fn enabling_survives_a_reload() {
        let db = test_db();
        save_config(
            &db,
            &JunkConfig {
                enabled: true,
                phishing_enabled: true,
                flagged_action: FlaggedAction::Hide,
            },
        )
        .expect("save");

        let loaded = get_config(&db);
        assert!(loaded.enabled);
        assert!(loaded.phishing_enabled);
        assert_eq!(loaded.flagged_action, FlaggedAction::Hide);
    }

    #[test]
    fn switching_back_off_is_not_read_as_unset() {
        // Guards the shape of the parse: an explicit "false" and a missing key
        // must both mean off, but a flipped predicate would make one of them
        // silently mean on.
        let db = test_db();
        save_config(
            &db,
            &JunkConfig {
                enabled: true,
                phishing_enabled: false,
                flagged_action: FlaggedAction::Dim,
            },
        )
        .expect("save on");
        save_config(
            &db,
            &JunkConfig {
                enabled: false,
                phishing_enabled: false,
                flagged_action: FlaggedAction::Dim,
            },
        )
        .expect("save off");

        assert!(!get_config(&db).enabled);
    }
}
