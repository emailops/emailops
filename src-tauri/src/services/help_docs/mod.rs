//! "EmailOps help": answering questions about the app itself — settings,
//! features, installation, troubleshooting — from the user guides bundled in
//! the binary, inside the same chat that answers questions about the
//! mailbox. See `MODULE.md`.

pub mod corpus;
pub mod index;
pub mod nav;
pub mod prompt;
pub mod retrieval;

pub use index::{ensure_embeddings, ensure_index, ensure_text_index};
pub use nav::{help_link_line, plan_help_link_fallback, plan_help_navigation, NavTarget};
pub use prompt::render_help_block;
pub use retrieval::{lookup_help, HelpSource, HELP_TOP_K};
