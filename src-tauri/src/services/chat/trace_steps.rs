//! A chat turn's trace as one ordered list of steps.
//!
//! Pure. Every place that shows a trace — the reasoning panel, `emailops-cli
//! chat --trace`, the eval report — walks the list this module builds, so the
//! turn reads the same everywhere and a new step is added in one place.

use crate::models::{CacheAction, CacheActionKind, ChatTrace, KvCacheStats, LlmCallTrace, TraceStep};

/// The steps of `trace`, in execution order:
///
/// 1. the route decision;
/// 2. the query planner's call(s), which decide the route and any search;
/// 3. mailbox RAG, then the guides lookup, which run on the planner's verdict;
/// 4. pre-seeded shortcut tools (round < 0), which run before the tool loop;
/// 5. each loop LLM call followed by the tools it issued;
/// 6. any tool no round claimed, last rather than hidden.
///
/// Tools issued by `tool_round` R are the ones with `round == R`, drained with
/// a forward cursor so an early round cannot claim a later tool (and a
/// salvaged call a round reported as `tool_calls_requested = 0` still lands
/// after it). Traces persisted before tools carried a round deserialise every
/// tool to 0; when that is the case and the rounds requested at least as many
/// tools as there are, each round takes its `tool_calls_requested` slice
/// instead. A `final_stream` never owns tools.
pub fn plan_steps(trace: &ChatTrace) -> Vec<TraceStep> {
    // A research turn opens with its summary, so the mode reads before the
    // steps it changed.
    let mut steps = if trace.research.is_some() {
        vec![TraceStep::Research, TraceStep::Route]
    } else {
        vec![TraceStep::Route]
    };
    let llm_step = |index: usize| {
        let call = &trace.llm_calls[index];
        TraceStep::Llm {
            index,
            kv_cache: kv_cache_stats(call),
            cache_action: cache_action(call),
        }
    };

    for (i, call) in trace.llm_calls.iter().enumerate() {
        if call.kind == "planner" {
            steps.push(llm_step(i));
        }
    }
    if trace.retrieval.is_some() {
        steps.push(TraceStep::Retrieval);
    }
    if trace.help.as_ref().is_some_and(|h| h.candidates > 0) {
        steps.push(TraceStep::Help);
    }

    let (preseeded, looped): (Vec<usize>, Vec<usize>) =
        (0..trace.tool_calls.len()).partition(|&i| trace.tool_calls[i].round < 0);
    steps.extend(preseeded.into_iter().map(|index| TraceStep::Tool { index }));

    let requested: i64 = trace.llm_calls.iter().map(|c| i64::from(c.tool_calls_requested)).sum();
    let legacy = !looped.is_empty()
        && looped.iter().all(|&i| trace.tool_calls[i].round == 0)
        && requested >= looped.len() as i64;

    let mut cursor = 0;
    let mut consumed = vec![false; looped.len()];
    for (i, call) in trace.llm_calls.iter().enumerate() {
        if call.kind == "planner" {
            continue;
        }
        steps.push(llm_step(i));
        if call.kind != "tool_round" {
            continue;
        }
        if legacy {
            let take = usize::try_from(call.tool_calls_requested).unwrap_or(0);
            for _ in 0..take {
                if cursor >= looped.len() {
                    break;
                }
                steps.push(TraceStep::Tool { index: looped[cursor] });
                consumed[cursor] = true;
                cursor += 1;
            }
        } else {
            let mut j = cursor;
            while j < looped.len() {
                let round = trace.tool_calls[looped[j]].round;
                if round == call.round {
                    steps.push(TraceStep::Tool { index: looped[j] });
                    consumed[j] = true;
                    cursor = j + 1;
                } else if round > call.round {
                    break;
                }
                j += 1;
            }
        }
    }

    for (j, &index) in looped.iter().enumerate() {
        if !consumed[j] {
            steps.push(TraceStep::Tool { index });
        }
    }
    steps
}

/// `trace` with its `steps` filled in (a no-op when they already are).
pub fn with_steps(mut trace: ChatTrace) -> ChatTrace {
    if trace.steps.is_empty() {
        trace.steps = plan_steps(&trace);
    }
    trace
}

/// KV-cache reuse for one call, or `None` when the provider does not report
/// it. A reported 0/N is a real cold prefill and is returned.
pub fn kv_cache_stats(call: &LlmCallTrace) -> Option<KvCacheStats> {
    let cached = call.cached_prompt_tokens?;
    let total = call.prompt_tokens.filter(|t| *t > 0)?;
    Some(KvCacheStats {
        cached,
        total,
        pct: ((f64::from(cached) / f64::from(total)) * 100.0).round() as u32,
    })
}

/// What happened to the system anchor (seq 2) across one call.
fn describe_anchor(before: u32, after: u32) -> String {
    match (before, after) {
        (0, 0) => "no anchor seeded (sys_tok=0 — system prefix not detected this call)".into(),
        (0, a) => format!("anchor seeded for the first time ({a} tok)"),
        (b, 0) => format!("anchor wiped (was {b} tok) and NOT reseeded — runtime returned sys_tok=0"),
        (b, a) if b == a => format!("anchor unchanged ({a} tok)"),
        (b, a) => format!("anchor resized {b}→{a} tok {}", if b < a { '↑' } else { '↓' }),
    }
}

/// What one call did to the prompt cache, or `None` without a prefix plan
/// (HTTP providers, traces from before the plan was recorded).
pub fn cache_action(call: &LlmCallTrace) -> Option<CacheAction> {
    let plan = call.prefix_plan.as_deref()?;
    // One-shot calls (planner, research) use the auxiliary prefix slot, not
    // the chat anchor: their plan is the slot's own verdict.
    if call.kind == "planner" || call.kind.starts_with("research_") {
        let (kind, detail) = match plan {
            "Reuse" => (CacheActionKind::Extend, "one-shot slot: instructions reused"),
            "Reseed" => (CacheActionKind::ColdFresh, "one-shot slot: instructions decoded"),
            _ => (
                CacheActionKind::ColdFresh,
                "one-shot slot not used (window below 16k or evicted)",
            ),
        };
        return Some(CacheAction {
            kind,
            detail: detail.to_string(),
        });
    }
    let before = call.sys_cached_before.unwrap_or(0);
    let after = call.sys_cached_after.unwrap_or(0);
    let dropped = call.dropped_front_tokens.unwrap_or(0);
    // Front-truncation rewrites the leading bytes, so no plan can reuse the
    // cache: name it as the cause whatever the plan was.
    if dropped > 0 {
        return Some(CacheAction {
            kind: CacheActionKind::Wiped,
            detail: format!(
                "out of context: front-truncated by {dropped} tokens — leading bytes rewrote, cache cannot follow (anchor not seeded)"
            ),
        });
    }
    let anchor = describe_anchor(before, after);
    let (kind, detail) = match plan {
        "Extend" => (
            if before == 0 && after > 0 {
                CacheActionKind::ColdFresh
            } else {
                CacheActionKind::Extend
            },
            format!("extended seq 0 · {anchor}"),
        ),
        "RestartFromAnchor" => (
            CacheActionKind::AnchorHit,
            format!("anchor hit: reused {after} tokens (cross-conversation) · {anchor}"),
        ),
        _ => (
            if before > 0 {
                CacheActionKind::Wiped
            } else {
                CacheActionKind::ColdFresh
            },
            format!("cold prefill · {anchor}"),
        ),
    };
    Some(CacheAction { kind, detail })
}

/// Plain-English details of a step — the numbers behind it. Pairs with
/// [`step_label`] in the CLI and the eval report.
pub fn step_detail(trace: &ChatTrace, step: &TraceStep) -> String {
    match step {
        TraceStep::Route => {
            let mode = match trace.route.mode {
                crate::models::RouteMode::RagFirst => "rag_first",
                crate::models::RouteMode::ToolsFirst => "tools_first",
            };
            if trace.route.reason.is_empty() {
                mode.to_string()
            } else {
                format!("{mode} · {}", trace.route.reason)
            }
        }
        TraceStep::Research => match &trace.research {
            Some(r) => {
                let mut d = format!(
                    "gathered {} by filter + {} by meaning · read {} of {} · {} findings from {} emails",
                    r.search_hits, r.semantic_hits, r.emails_analyzed, r.planned_emails, r.findings, r.relevant_emails
                );
                if r.condense_calls > 0 {
                    d.push_str(&format!(" · {} condense calls", r.condense_calls));
                }
                if r.failed_batches > 0 {
                    let plural = if r.failed_batches == 1 { "batch" } else { "batches" };
                    d.push_str(&format!(" · {} {plural} failed", r.failed_batches));
                }
                if r.stopped {
                    d.push_str(" · cancelled by the user");
                }
                d
            }
            None => String::new(),
        },
        TraceStep::Retrieval => match &trace.retrieval {
            Some(r) => format!(
                "{} vec + {} fts → top {} · {} ms{}",
                r.vector_hits,
                r.fts_hits,
                r.fused_top_k,
                r.elapsed_ms,
                if r.vector_fallback { " · vector fallback" } else { "" }
            ),
            None => String::new(),
        },
        TraceStep::Help => match &trace.help {
            Some(h) => {
                let mut parts = Vec::new();
                if let Some(sim) = h.top_similarity {
                    parts.push(format!("sim {sim:.2}"));
                }
                parts.push(format!("{} ms", h.elapsed_ms));
                parts.join(" · ")
            }
            None => String::new(),
        },
        TraceStep::Llm {
            index,
            kv_cache,
            cache_action,
        } => {
            let Some(call) = trace.llm_calls.get(*index) else {
                return String::new();
            };
            let mut parts = vec![format!("{} ms", call.latency_ms)];
            if let Some(p) = call.prefill_ms {
                parts.push(format!("prefill {p} ms"));
            }
            if let Some(k) = kv_cache {
                parts.push(format!("KV cache {}/{} tok ({}%)", k.cached, k.total, k.pct));
            }
            if let Some(a) = cache_action {
                parts.push(a.detail.clone());
            }
            match call.tool_calls_requested {
                0 => {}
                1 => parts.push("1 tool call".into()),
                n => parts.push(format!("{n} tool calls")),
            }
            if call.failed {
                parts.push("FAILED".into());
            }
            parts.join(" · ")
        }
        TraceStep::Tool { index } => trace
            .tool_calls
            .get(*index)
            .map(|t| format!("{} ms · {} chars", t.elapsed_ms, t.result_chars))
            .unwrap_or_default(),
    }
}

/// Plain-English one-line label for a step — the CLI and the eval report
/// print these; the reasoning panel translates the step type instead.
pub fn step_label(trace: &ChatTrace, step: &TraceStep) -> String {
    match step {
        TraceStep::Route => {
            let r = &trace.route;
            if r.matched_keywords.is_empty() {
                format!("route: {}", r.classifier)
            } else {
                format!("route: {} (matched: {})", r.classifier, r.matched_keywords.join(", "))
            }
        }
        TraceStep::Research => match &trace.research {
            Some(r) => format!("research ({} emails, {} batches)", r.emails_analyzed, r.batches),
            None => "research".into(),
        },
        TraceStep::Retrieval => "RAG retrieval".into(),
        TraceStep::Help => match &trace.help {
            Some(h) => format!("guides ({} of {} sections)", h.included, h.candidates),
            None => "guides".into(),
        },
        TraceStep::Llm { index, .. } => match trace.llm_calls.get(*index) {
            Some(c) if c.kind == "planner" => "planner".into(),
            Some(c) if c.kind == "final_stream" => "answer".into(),
            Some(c) if c.kind == "tool_round" => format!("llm round {}", c.round),
            Some(c) => format!("llm call ({})", c.kind),
            None => "llm call".into(),
        },
        TraceStep::Tool { index } => trace
            .tool_calls
            .get(*index)
            .map(|t| t.name.clone())
            .unwrap_or_else(|| "tool".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{HelpTrace, RetrievalTrace, RouteDecision, RouteMode, ToolCallTrace};

    fn llm(kind: &str, round: i32, requested: i32) -> LlmCallTrace {
        LlmCallTrace {
            kind: kind.into(),
            round,
            latency_ms: 1,
            tool_calls_requested: requested,
            failed: false,
            prompt_tokens: None,
            prefill_ms: None,
            cached_prompt_tokens: None,
            prefix_plan: None,
            sys_cached_before: None,
            sys_cached_after: None,
            system_prefix_tokens: None,
            stable_tokens: None,
            dropped_front_tokens: None,
            input: None,
            output: None,
        }
    }

    fn tool(name: &str, round: i32) -> ToolCallTrace {
        ToolCallTrace {
            name: name.into(),
            round,
            arguments: serde_json::json!({}),
            result_preview: String::new(),
            result_chars: 0,
            elapsed_ms: 1,
        }
    }

    fn trace(classifier: &str, llm_calls: Vec<LlmCallTrace>, tool_calls: Vec<ToolCallTrace>) -> ChatTrace {
        ChatTrace {
            route: RouteDecision {
                mode: RouteMode::ToolsFirst,
                reason: String::new(),
                matched_keywords: vec![],
                classifier: classifier.into(),
            },
            retrieval: None,
            tool_calls,
            model: "m".into(),
            total_elapsed_ms: 1,
            tool_loop_ms: 1,
            llm_streaming_ms: None,
            llm_calls,
            help: None,
            research: None,
            steps: vec![],
        }
    }

    fn retrieval() -> RetrievalTrace {
        serde_json::from_value(serde_json::json!({
            "vectorHits": 20, "ftsHits": 30, "fusedTopK": 9, "elapsedMs": 7,
            "ftsSearchMs": 1, "fetchMs": 0, "expansionMs": 0
        }))
        .expect("retrieval fixture")
    }

    /// The steps as short tags — `llm:tool_round/0`, `tool:search_emails` — so
    /// an ordering assertion reads like the timeline it checks.
    fn tags(t: &ChatTrace) -> Vec<String> {
        plan_steps(t)
            .iter()
            .map(|s| match s {
                TraceStep::Route => "route".into(),
                TraceStep::Research => "research".into(),
                TraceStep::Retrieval => "rag".into(),
                TraceStep::Help => "help".into(),
                TraceStep::Llm { index, .. } => {
                    let c = &t.llm_calls[*index];
                    format!("llm:{}/{}", c.kind, c.round)
                }
                TraceStep::Tool { index } => format!("tool:{}", t.tool_calls[*index].name),
            })
            .collect()
    }

    // ── Order (ported from src/lib/reasoningTrace.ts `buildFlow`) ────────

    #[test]
    fn a_one_shot_call_reports_its_prefix_slot_not_the_chat_anchor() {
        // Planner and research calls run on the auxiliary slot, which reports
        // Reuse / Reseed / Bypass. Reading that as a chat plan printed "cold
        // prefill · no anchor seeded (sys_tok=0 …)" under every batch.
        let mut c = llm("research_map", 0, 0);
        c.prefix_plan = Some("Reuse".into());
        let a = cache_action(&c).expect("an action");
        assert_eq!(a.kind, CacheActionKind::Extend);
        assert!(a.detail.contains("instructions reused"), "{}", a.detail);
        assert!(!a.detail.contains("anchor"), "{}", a.detail);

        c.prefix_plan = Some("Reseed".into());
        assert_eq!(cache_action(&c).map(|a| a.kind), Some(CacheActionKind::ColdFresh));
        c.prefix_plan = Some("Bypass".into());
        let a = cache_action(&c).expect("an action");
        assert!(a.detail.contains("not used"), "{}", a.detail);
    }

    #[test]
    fn a_research_turn_reads_header_router_planner_gather_map_condense_reduce() {
        let mut t = trace(
            "heuristic",
            vec![
                llm("planner", -2, 0),
                llm("research_map", 0, 0),
                llm("research_map", 1, 0),
                llm("research_condense", 0, 0),
                llm("research_reduce", -1, 0),
            ],
            vec![tool("search_emails", -3), tool("search_emails", -3)],
        );
        t.research = Some(crate::models::ResearchTrace {
            emails_analyzed: 20,
            batches: 2,
            ..Default::default()
        });
        assert_eq!(
            tags(&t),
            [
                "research",
                "route",
                "llm:planner/-2",
                "tool:search_emails",
                "tool:search_emails",
                "llm:research_map/0",
                "llm:research_map/1",
                "llm:research_condense/0",
                "llm:research_reduce/-1"
            ]
        );
    }

    #[test]
    fn research_step_label_and_detail_carry_the_counts() {
        let mut t = trace("planner", vec![], vec![]);
        t.research = Some(crate::models::ResearchTrace {
            planned_emails: 70,
            search_hits: 30,
            semantic_hits: 40,
            emails_analyzed: 60,
            stopped: true,
            batches: 6,
            failed_batches: 1,
            findings: 25,
            relevant_emails: 18,
            ..Default::default()
        });
        assert_eq!(step_label(&t, &TraceStep::Research), "research (60 emails, 6 batches)");
        let detail = step_detail(&t, &TraceStep::Research);
        assert!(detail.contains("30 by filter + 40 by meaning"), "{detail}");
        assert!(detail.contains("read 60 of 70"), "{detail}");
        assert!(detail.contains("cancelled by the user"), "{detail}");
        assert!(detail.contains("25 findings from 18 emails"), "{detail}");
        assert!(detail.contains("1 batch failed"), "{detail}");
    }

    #[test]
    fn the_route_always_comes_first() {
        assert_eq!(tags(&trace("heuristic", vec![], vec![])), ["route"]);
    }

    #[test]
    fn a_preseeded_shortcut_tool_precedes_the_round_that_consumed_it() {
        let t = trace(
            "heuristic",
            vec![llm("tool_round", 0, 0)],
            vec![tool("search_emails", -1)],
        );
        assert_eq!(tags(&t), ["route", "tool:search_emails", "llm:tool_round/0"]);
    }

    #[test]
    fn the_planner_leads_ahead_of_the_search_it_preseeded() {
        let t = trace(
            "planner",
            vec![llm("tool_round", 0, 0), llm("planner", -2, 0)],
            vec![tool("search_emails", -1)],
        );
        assert_eq!(
            tags(&t),
            ["route", "llm:planner/-2", "tool:search_emails", "llm:tool_round/0"]
        );
    }

    #[test]
    fn rag_and_the_guides_run_after_the_planner_and_before_any_tool() {
        let mut t = trace(
            "planner",
            vec![llm("planner", -2, 0), llm("tool_round", 0, 1)],
            vec![tool("get_thread", 0)],
        );
        t.retrieval = Some(retrieval());
        t.help = Some(HelpTrace {
            candidates: 24,
            included: 2,
            ..Default::default()
        });
        assert_eq!(
            tags(&t),
            [
                "route",
                "llm:planner/-2",
                "rag",
                "help",
                "llm:tool_round/0",
                "tool:get_thread"
            ]
        );
    }

    #[test]
    fn a_help_lookup_with_no_candidates_is_not_a_step() {
        let mut t = trace("heuristic", vec![], vec![]);
        t.help = Some(HelpTrace::default());
        assert_eq!(tags(&t), ["route"]);
    }

    #[test]
    fn a_loop_tool_follows_the_round_that_requested_it() {
        let t = trace(
            "heuristic",
            vec![llm("tool_round", 0, 1), llm("tool_round", 1, 0)],
            vec![tool("search_emails", 0)],
        );
        assert_eq!(
            tags(&t),
            ["route", "llm:tool_round/0", "tool:search_emails", "llm:tool_round/1"]
        );
    }

    #[test]
    fn multi_round_tools_follow_the_round_that_issued_each() {
        let t = trace(
            "heuristic",
            vec![
                llm("tool_round", 0, 1),
                llm("tool_round", 1, 1),
                llm("tool_round", 2, 0),
            ],
            vec![tool("search_emails", 0), tool("get_thread", 1)],
        );
        assert_eq!(
            tags(&t),
            [
                "route",
                "llm:tool_round/0",
                "tool:search_emails",
                "llm:tool_round/1",
                "tool:get_thread",
                "llm:tool_round/2"
            ]
        );
    }

    #[test]
    fn legacy_traces_slice_tools_by_the_count_each_round_requested() {
        // Persisted before `round` existed: every tool deserialises to 0.
        let t = trace(
            "heuristic",
            vec![
                llm("tool_round", 0, 1),
                llm("tool_round", 1, 2),
                llm("tool_round", 2, 0),
            ],
            vec![tool("a", 0), tool("b", 0), tool("c", 0)],
        );
        assert_eq!(
            tags(&t),
            [
                "route",
                "llm:tool_round/0",
                "tool:a",
                "llm:tool_round/1",
                "tool:b",
                "tool:c",
                "llm:tool_round/2"
            ]
        );
    }

    #[test]
    fn a_final_stream_never_owns_tools() {
        let t = trace(
            "heuristic",
            vec![llm("tool_round", 0, 1), llm("final_stream", -1, 0)],
            vec![tool("get_thread", 0)],
        );
        assert_eq!(
            tags(&t),
            ["route", "llm:tool_round/0", "tool:get_thread", "llm:final_stream/-1"]
        );
    }

    #[test]
    fn salvaged_tools_follow_their_round_even_when_it_requested_none() {
        let t = trace(
            "heuristic",
            vec![llm("tool_round", 0, 0), llm("tool_round", 1, 0)],
            vec![tool("search_emails", 0), tool("get_thread", 0)],
        );
        assert_eq!(
            tags(&t),
            [
                "route",
                "llm:tool_round/0",
                "tool:search_emails",
                "tool:get_thread",
                "llm:tool_round/1"
            ]
        );
    }

    #[test]
    fn tools_no_round_claimed_are_surfaced_at_the_end() {
        let t = trace("heuristic", vec![llm("tool_round", 0, 0)], vec![tool("orphan", 5)]);
        assert_eq!(tags(&t), ["route", "llm:tool_round/0", "tool:orphan"]);
    }

    #[test]
    fn with_steps_keeps_steps_already_present() {
        let mut t = trace("heuristic", vec![], vec![]);
        t.steps = vec![TraceStep::Help];
        assert_eq!(with_steps(t).steps, vec![TraceStep::Help]);
        assert_eq!(
            with_steps(trace("heuristic", vec![], vec![])).steps,
            vec![TraceStep::Route]
        );
    }

    #[test]
    fn llm_steps_carry_their_cache_facts() {
        let mut c = llm("tool_round", 0, 0);
        c.prompt_tokens = Some(200);
        c.cached_prompt_tokens = Some(150);
        c.prefix_plan = Some("Extend".into());
        c.sys_cached_before = Some(80);
        c.sys_cached_after = Some(80);
        let t = trace("heuristic", vec![c], vec![]);
        match &plan_steps(&t)[1] {
            TraceStep::Llm {
                kv_cache, cache_action, ..
            } => {
                assert_eq!(kv_cache.as_ref().map(|k| k.pct), Some(75));
                assert_eq!(cache_action.as_ref().map(|a| a.kind), Some(CacheActionKind::Extend));
            }
            other => panic!("expected an llm step, got {other:?}"),
        }
    }

    // ── KV cache (ported from `kvCacheStats` / `cacheAction`) ───────────

    fn cached(prompt: Option<u32>, cached: Option<u32>) -> LlmCallTrace {
        let mut c = llm("tool_round", 0, 0);
        c.prompt_tokens = prompt;
        c.cached_prompt_tokens = cached;
        c
    }

    #[test]
    fn kv_cache_stats_report_the_share_served_from_cache() {
        assert_eq!(
            kv_cache_stats(&cached(Some(8785), Some(7856))),
            Some(KvCacheStats {
                cached: 7856,
                total: 8785,
                pct: 89
            })
        );
        assert_eq!(kv_cache_stats(&cached(Some(500), Some(0))).map(|k| k.pct), Some(0));
        assert_eq!(
            kv_cache_stats(&cached(Some(500), None)),
            None,
            "HTTP providers report nothing"
        );
        assert_eq!(kv_cache_stats(&cached(Some(0), Some(0))), None, "no divide by zero");
        assert_eq!(kv_cache_stats(&cached(None, Some(10))), None);
    }

    fn plan(plan: Option<&str>, before: u32, after: u32, dropped: u32) -> LlmCallTrace {
        let mut c = llm("tool_round", 0, 0);
        c.prefix_plan = plan.map(str::to_string);
        c.sys_cached_before = Some(before);
        c.sys_cached_after = Some(after);
        c.dropped_front_tokens = Some(dropped);
        c
    }

    fn action(c: &LlmCallTrace) -> (CacheActionKind, String) {
        let a = cache_action(c).expect("an action");
        (a.kind, a.detail)
    }

    #[test]
    fn no_prefix_plan_means_no_cache_action() {
        assert_eq!(cache_action(&plan(None, 0, 0, 0)), None);
    }

    #[test]
    fn an_in_conversation_extend_keeps_the_anchor() {
        assert_eq!(
            action(&plan(Some("Extend"), 7113, 7113, 0)),
            (
                CacheActionKind::Extend,
                "extended seq 0 · anchor unchanged (7113 tok)".into()
            )
        );
    }

    #[test]
    fn an_extend_that_seeds_the_anchor_is_a_first_time_event() {
        assert_eq!(
            action(&plan(Some("Extend"), 0, 7113, 0)),
            (
                CacheActionKind::ColdFresh,
                "extended seq 0 · anchor seeded for the first time (7113 tok)".into()
            )
        );
    }

    #[test]
    fn a_restart_from_anchor_is_the_cross_conversation_hit() {
        assert_eq!(
            action(&plan(Some("RestartFromAnchor"), 7113, 7113, 0)),
            (
                CacheActionKind::AnchorHit,
                "anchor hit: reused 7113 tokens (cross-conversation) · anchor unchanged (7113 tok)".into()
            )
        );
    }

    #[test]
    fn a_cold_prefill_over_an_anchor_wiped_it() {
        assert_eq!(
            action(&plan(Some("ColdPrefill"), 7113, 6900, 0)),
            (
                CacheActionKind::Wiped,
                "cold prefill · anchor resized 7113→6900 tok ↓".into()
            )
        );
    }

    #[test]
    fn the_first_cold_prefill_is_fresh() {
        assert_eq!(
            action(&plan(Some("ColdPrefill"), 0, 7113, 0)).0,
            CacheActionKind::ColdFresh
        );
    }

    #[test]
    fn an_anchor_wiped_and_not_reseeded_is_flagged() {
        assert_eq!(
            action(&plan(Some("ColdPrefill"), 7113, 0, 0)).1,
            "cold prefill · anchor wiped (was 7113 tok) and NOT reseeded — runtime returned sys_tok=0"
        );
    }

    #[test]
    fn a_missing_system_prefix_is_explained() {
        assert_eq!(
            action(&plan(Some("Extend"), 0, 0, 0)).1,
            "extended seq 0 · no anchor seeded (sys_tok=0 — system prefix not detected this call)"
        );
    }

    #[test]
    fn front_truncation_is_named_as_the_cause() {
        assert_eq!(
            action(&plan(Some("Extend"), 7113, 7113, 412)),
            (
                CacheActionKind::Wiped,
                "out of context: front-truncated by 412 tokens — leading bytes rewrote, cache cannot follow (anchor not seeded)"
                    .into()
            )
        );
    }

    // ── Labels (the CLI and the eval report) ────────────────────────────

    fn labels(t: &ChatTrace) -> Vec<String> {
        plan_steps(t).iter().map(|s| step_label(t, s)).collect()
    }

    #[test]
    fn labels_name_each_step_in_plain_english() {
        let mut t = trace(
            "planner",
            vec![
                llm("planner", -2, 0),
                llm("tool_round", 0, 1),
                llm("tool_round", 1, 0),
                llm("final_stream", -1, 0),
            ],
            vec![tool("get_thread", 0)],
        );
        t.retrieval = Some(retrieval());
        t.help = Some(HelpTrace {
            candidates: 24,
            included: 2,
            ..Default::default()
        });
        assert_eq!(
            labels(&t),
            [
                "route: planner",
                "planner",
                "RAG retrieval",
                "guides (2 of 24 sections)",
                "llm round 0",
                "get_thread",
                "llm round 1",
                "answer"
            ]
        );
    }

    fn details(t: &ChatTrace) -> Vec<String> {
        plan_steps(t).iter().map(|s| step_detail(t, s)).collect()
    }

    #[test]
    fn details_carry_the_numbers_behind_each_step() {
        let mut planner = llm("planner", -2, 0);
        planner.latency_ms = 219;
        planner.prefill_ms = Some(8);
        planner.prompt_tokens = Some(1777);
        planner.cached_prompt_tokens = Some(1757);
        planner.prefix_plan = Some("Reuse".into());
        let mut round = llm("tool_round", 0, 1);
        round.latency_ms = 1100;
        round.failed = true;
        let mut t = trace("planner", vec![planner, round], vec![tool("list_lenses", 0)]);
        t.route.mode = RouteMode::RagFirst;
        t.route.reason = "planner found no single search".into();
        t.retrieval = Some(retrieval());
        t.help = Some(HelpTrace {
            candidates: 24,
            included: 2,
            top_similarity: Some(0.812),
            elapsed_ms: 7,
            ..Default::default()
        });
        t.tool_calls[0].elapsed_ms = 3;
        t.tool_calls[0].result_chars = 246;
        assert_eq!(
            details(&t),
            [
                "rag_first · planner found no single search",
                "219 ms · prefill 8 ms · KV cache 1757/1777 tok (99%) · one-shot slot: instructions reused",
                "20 vec + 30 fts → top 9 · 7 ms",
                "sim 0.81 · 7 ms",
                "1100 ms · 1 tool call · FAILED",
                "3 ms · 246 chars"
            ]
        );
    }

    #[test]
    fn the_route_label_carries_what_the_heuristic_matched() {
        let mut t = trace("heuristic", vec![], vec![]);
        t.route.matched_keywords = vec!["hoy".into(), "2026".into()];
        assert_eq!(labels(&t), ["route: heuristic (matched: hoy, 2026)"]);
    }
}
