// Form-filling harness.
//
// `forms.fill` turns a request like "crea una lens de facturas con importe y
// fecha" into the fields of an app form, which the user then reviews and saves.
// Everything the user sees on that form comes from one completion, so its
// quality is cheap to measure directly — one small call per case — instead of
// being inferred from a full chat answer.
//
// What the cases measure, in the order they matter:
//   1. completeness — a form missing a required field cannot be submitted;
//   2. coverage — one column per thing the request named;
//   3. restraint — a key the request never implied silently narrows the result;
//   4. typing — `currency` on an amount column, not `string`.

pub mod case_loader;
pub mod metrics;
pub mod runner;
