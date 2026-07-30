// `junk_eval` — measurement gate for the junk detector (spam / phishing / graymail).
//
// Fully synthetic and fully deterministic: no model, no network, no database.
// Cases live in `src-tauri/evals/junk/cases/` and contain no personal-mailbox
// content, so this runs in milliseconds and is safe in CI.
//
// Exits non-zero when a gate is blown. The gates are asymmetric on purpose: a
// missed spam message is a warning, a false positive on real mail fails the
// build.
//
// Usage:
//   cargo run --features eval --example junk_eval
//   cargo run --features eval --example junk_eval -- --case phish-bec-lookalike-domain-reply-to-mismatch
//   cargo run --features eval --example junk_eval -- --tier smoke
//
// Flags:
//   --cases-dir <dir>   Case directory. Default: src-tauri/evals/junk/cases.
//   --tier <name>       Run only cases in this tier.
//   --case <id>         Run a single case by id.
//   --out <dir>         Report dir. Default: reports/evaluations/junk.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use emailops_lib::evals::junk::metrics::Gates;
use emailops_lib::evals::junk::runner::{run, JunkEvalConfig};

#[derive(Parser, Debug)]
#[command(
    name = "junk_eval",
    about = "Score the junk detector against the synthetic corpus and enforce the FP budget."
)]
struct Args {
    #[arg(long)]
    cases_dir: Option<PathBuf>,

    #[arg(long)]
    tier: Option<String>,

    #[arg(long = "case")]
    case: Option<String>,

    #[arg(long)]
    out: Option<PathBuf>,
}

fn main() -> ExitCode {
    let args = Args::parse();

    let cases_dir = args
        .cases_dir
        .unwrap_or_else(|| PathBuf::from("src-tauri/evals/junk/cases"));
    let out_dir = args.out.unwrap_or_else(|| PathBuf::from("reports/evaluations/junk"));

    let cfg = JunkEvalConfig {
        cases_dir,
        out_dir,
        tier: args.tier,
        case_filter: args.case,
        gates: Gates::default(),
    };

    match run(cfg) {
        Ok(summary) => {
            eprintln!("[junk-eval] report  → {}", summary.report_path.display());
            eprintln!("[junk-eval] metrics → {}", summary.metrics_path.display());
            if summary.metrics.gates_passed {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(e) => {
            eprintln!("[junk-eval] ERROR: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_the_public_synthetic_corpus() {
        // Must never default to private-evals/: this suite is the one that runs
        // in CI, and CI has no access to (and no business with) real mailbox data.
        let args = Args::try_parse_from(["junk_eval"]).expect("parse default args");
        assert!(args.cases_dir.is_none());
        assert!(args.case.is_none());
    }

    #[test]
    fn accepts_a_single_case_filter() {
        let args = Args::try_parse_from(["junk_eval", "--case", "phish-001"]).expect("parse case filter");
        assert_eq!(args.case.as_deref(), Some("phish-001"));
    }
}
