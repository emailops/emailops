//! Thread-dedup: collapse a ranked list of email ids to one entry per thread,
//! keeping the highest-scored email per thread.
//!
//! Done as a free function over a `get_thread_id` closure so callers that
//! already have email metadata in memory (chat/search) don't have to roundtrip
//! the DB just to look up thread ids. The agent_search consumer can pass a
//! closure that consults a small `HashMap` it built from a batched
//! `db.get_emails_by_ids` call.
//!
//! Emails whose thread id can't be resolved are kept as their own pseudo-thread
//! so an orphaned vec0 row doesn't silently disappear from the ranking.

/// Collapse `ranked` to the best-scored email per thread, preserving original
/// rank order across surviving entries (i.e. the kept email's original
/// position in `ranked` determines the output order).
pub fn dedup_by_thread<F>(ranked: Vec<(String, f32)>, mut get_thread_id: F) -> Vec<(String, f32)>
where
    F: FnMut(&str) -> Option<String>,
{
    use std::collections::HashMap;

    // First pass: find the best score per thread.
    let mut best_per_thread: HashMap<String, f32> = HashMap::new();
    let mut thread_key_for: Vec<(String, f32, String)> = Vec::with_capacity(ranked.len());
    for (id, score) in ranked.iter() {
        let thread_key = get_thread_id(id).unwrap_or_else(|| format!("__orphan__:{}", id));
        let entry = best_per_thread.entry(thread_key.clone()).or_insert(f32::NEG_INFINITY);
        if *score > *entry {
            *entry = *score;
        }
        thread_key_for.push((id.clone(), *score, thread_key));
    }

    // Second pass: walk `ranked` in order, keep only the email whose score
    // equals the best for its thread, and dedup on first occurrence.
    let mut seen_threads: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<(String, f32)> = Vec::new();
    for (id, score, thread_key) in thread_key_for.into_iter() {
        if !seen_threads.contains(&thread_key) && (score - best_per_thread[&thread_key]).abs() < f32::EPSILON {
            seen_threads.insert(thread_key);
            out.push((id, score));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn thread_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn dedup_by_thread_keeps_highest_scored_per_thread() {
        // Three emails: a,b in thread T1; c in thread T2. b has a higher score
        // than a, so b should survive — and c is kept since it's in its own
        // thread.
        let m = thread_map(&[("a", "T1"), ("b", "T1"), ("c", "T2")]);
        let ranked = vec![("a".to_string(), 0.5), ("b".to_string(), 0.9), ("c".to_string(), 0.3)];
        let out = dedup_by_thread(ranked, |id| m.get(id).cloned());
        let ids: Vec<&str> = out.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["b", "c"]);
    }

    #[test]
    fn dedup_by_thread_orphan_ids_survive() {
        // No thread mapping → each gets its own pseudo-thread → both kept.
        let m: HashMap<String, String> = HashMap::new();
        let ranked = vec![("a".to_string(), 0.5), ("b".to_string(), 0.4)];
        let out = dedup_by_thread(ranked, |id| m.get(id).cloned());
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn dedup_by_thread_preserves_rank_order() {
        // Ranked order: a (T1), c (T2), b (T1). b has higher score than a,
        // but a appears first. We keep the *highest-scored* per thread → b.
        // Output order should be: c (T2, only one), then b (T1, the winner).
        let m = thread_map(&[("a", "T1"), ("b", "T1"), ("c", "T2")]);
        let ranked = vec![("a".to_string(), 0.5), ("c".to_string(), 0.4), ("b".to_string(), 0.9)];
        let out = dedup_by_thread(ranked, |id| m.get(id).cloned());
        let ids: Vec<&str> = out.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["c", "b"]);
    }
}
