//! Fillable app forms: the declarative field definitions the chat hands to the
//! model, and the parser that maps a model reply back onto them.
//!
//! See `MODULE.md` for the shape of the feature and why the form definition
//! never enters the chat system prompt.

pub mod filler;
pub mod registry;

pub use filler::{parse_fill, FillError, FormFill};
pub use registry::{catalog, lookup, FieldDef, FieldKind, FormDef, FORMS, LENS_CREATE};
