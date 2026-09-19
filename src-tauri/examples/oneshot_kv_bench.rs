// `oneshot_kv_bench` — what one-shot traffic costs the chat prompt cache.
//
// The classifier and the query planner run with `cache_prompt=false` so their
// prompts never evict the chat prefix. Every `make cli-*` run starts with an
// empty KV cache, so nothing so far could actually observe that: this binary
// keeps ONE process alive and probes a chat turn at rest, after a burst of
// classifications, with a planner call mixed in, queued behind a running
// batch, and again with the context window pinned to the 8 GB/16 GB tier.
//
// Usage:
//   cargo run --features eval --example oneshot_kv_bench -- [flags]
//   make bench-oneshot-kv

use std::path::PathBuf;

use clap::Parser;

use emailops_lib::evals::db_source::EvalDbMode;
use emailops_lib::evals::oneshot_kv::{run, KvBenchConfig};

#[derive(Parser, Debug)]
#[command(
    name = "oneshot_kv_bench",
    about = "Measure what classifier / planner one-shots cost the chat KV prefix, in one process.",
    long_about = None,
)]
struct Args {
    /// One-shot classifications per interleaved scenario.
    #[arg(long, default_value_t = 20)]
    classifications: usize,

    /// The account whose chat prompt is built.
    #[arg(long)]
    account: Option<String>,

    /// Skip the n_ctx=8192 scenarios (they respawn the actor, which is slow).
    #[arg(long, default_value_t = false)]
    skip_small_ctx: bool,

    #[arg(long)]
    prod_db: Option<PathBuf>,
}

fn main() {
    for p in [".env.local", ".env", "../.env.local", "../.env"] {
        if dotenvy::from_filename(p).is_ok() {
            break;
        }
    }

    let args = Args::parse();
    let cfg = KvBenchConfig {
        prod_db_path: args
            .prod_db
            .or_else(default_prod_db)
            .unwrap_or_else(|| PathBuf::from("emailops.db")),
        db_mode: EvalDbMode::CopyToTemp,
        account: args.account,
        classifications: args.classifications,
        small_ctx: !args.skip_small_ctx,
    };

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    let code = match rt.block_on(run(cfg)) {
        Ok(report) => match serde_json::to_string_pretty(&report) {
            Ok(json) => {
                println!("{json}");
                0
            }
            Err(e) => {
                eprintln!("[kv-bench] ERROR: {e}");
                1
            }
        },
        Err(e) => {
            eprintln!("[kv-bench] ERROR: {e}");
            1
        }
    };

    // Same exit path as the eval harnesses: ggml's Metal destructor aborts if
    // the process unwinds normally.
    emailops_lib::services::ai::shutdown_and_exit(code);
}

#[cfg(target_os = "macos")]
fn default_prod_db() -> Option<PathBuf> {
    dirs::home_dir().map(|h| {
        h.join("Library")
            .join("Application Support")
            .join("com.emailops.app")
            .join("emailops.db")
    })
}

#[cfg(not(target_os = "macos"))]
fn default_prod_db() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("com.emailops.app").join("emailops.db"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_parse_without_flags() {
        let args = Args::try_parse_from(["oneshot_kv_bench"]).expect("parse default args");
        assert_eq!(args.classifications, 20);
        assert!(!args.skip_small_ctx);
    }
}
