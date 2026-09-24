// Lens extraction harness.
//
// A Lens turns each email in its scope into one table row. This harness feeds
// synthetic emails through a built-in template's real scope and extractor and
// checks the row field by field against what a person reading the email would
// write down — the address and name of whoever filled in a contact form, the
// kind of request, which fields must stay empty. It also checks the template's
// scope picks the email up (or leaves it alone), because a perfect extraction
// the scope never reaches is worth nothing.

pub mod case_loader;
pub mod metrics;
pub mod runner;
