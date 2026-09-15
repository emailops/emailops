# Chat eval cases

`emailops-cli eval` (and `make verify`) runs every `cases/*.yaml` case through the real
chat turn against the synthetic demo DB, checks the deterministic anchors each case
declares (`expected_route`, `expected_tools_called`, `expected_answer_contains`…) and,
with `--judge`, scores the answer against its `expected_output` golden with the local
model as judge. Private cases keyed to a real mailbox live in `private-evals/chat/cases/`
and follow the same schema.

## `category`: what the user is asking for

`category` names the **user's intent**, not the engine path that serves it. One value
per case, from this list. Context, account and language are separate dimensions read
from the other fields (`thread_id`, `ambient_thread_id`, `account`, the language of
`question`), so any category can be exercised with or without an open email, on one
account or unified, in any UI language.

| `category` | The user wants… | Public example (demo persona) | What it exercises |
|---|---|---|---|
| `single_fact` | one concrete value that lives in one email | "what discount code did Fastmail send me?" | finding the right email, quoting without inventing; golden = the value, `answer_contains`, judge faithfulness |
| `topic_retrieval` | a list of emails about a topic, sender or period | "list all emails from Marisol as a markdown table" | coverage and precision of the set; every row linked `email://`; golden = expected ids |
| `email_summary` | one email condensed, translated or explained | "resume este correo" (open email), "traduceme este email al español" | reads the open/named email, answers inline, never drafts |
| `thread_summary` | a thread or a whole exchange with someone, in order | "en qué quedamos con Bahía Studio" | chronological order, latest state kept; golden = milestones |
| `period_summary` | what happened today / this week | "summarize today's emails", "que correos tengo hoy" | date window and category scope (CLI: Primary only), leaves an open thread when the question is mailbox-wide |
| `pending_actions` | what is expected from me, from one email or the mailbox | "list all my pending tasks" | mine vs theirs, each item linked to its email |
| `drafting` | a reply, a new email, or a rewrite of a draft | "escribe una respuesta a este correo", "draft a brief reply to the production bug" | `generate_email_draft` only when asked, `draft://` link in the answer, the body lives in the draft |
| `calendar` | meetings and agenda | "what is my next meeting?" | tools-first route, calendar-capable account, relative dates re-anchored to now |
| `people_companies` | who someone is, the people at a company, an address | "who do I know at Faro Logistics?" | aggregation by sender/domain, contact memory |
| `counts_aggregates` | how many, the first, the oldest, the latest | "muéstrame el primer correo que envié" | real totals, sort order, "just show it" does not draft |
| `attachments` | a file, or a value inside one | "send me Bahía Studio's May invoice" | attachment lookup, reading document content |
| `memory` | something the user said or learnt earlier | "what is my BorgBase customer number?" | memory facts and threads |
| `out_of_scope` | a question with no data, ambiguous, or not about the mailbox | "resume el correo de Juan" (three Juans), "what is the capital of Peru?" | says so or asks to disambiguate, no invented facts, no tool names leaked |

Cross-cutting dimensions every category should eventually cover:

- **Context**: no open email · open email that is the subject · open email that is *not* the
  subject (the question leaves the thread) · conversation bound to a thread.
- **Account**: one account · unified view · thread owned by another account.
- **Language**: question in Spanish or English; email bodies in another language.
- **Time**: relative references ("hoy", "last week", "next month") anchored to the run
  (`as_of`, the calendar re-anchoring in `ensure_demo_db.sh`).
- **Follow-up**: a second turn that depends on the first ("y el anterior", "open it").

## Writing a case

- Key public cases to the demo persona (`ulises@emailopslabs.dev`; calendar cases to
  `ulises.emailopslabs@gmail.com`). Anything from a real mailbox goes to `private-evals/`.
- Give every case deterministic anchors **and** an `expected_output` golden with
  `metrics: [answer_relevancy, faithfulness]`, so the judge has a reference.
- Anchor goldens on data the generator seeds (`scripts/generate_demo_db.py`); if the case
  needs rows the demo DB lacks, seed them there.
- Pick the `tier`: `smoke` (fast, run on every change), `full` (the rest), `lab`
  (exploratory, expected to be flaky while a behaviour is being worked on).
- Run it alone first: `make cli-eval ARGS="--case <id> --json --judge"`.
