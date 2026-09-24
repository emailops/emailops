//! The declarative description of every app form the chat can fill.
//!
//! One `FormDef` per form: a flat list of typed fields with a description
//! written *for the model*, not for the UI (the UI has its own i18n labels).
//! This is the JSON the filler hands to the LLM, and the contract the frontend
//! renders against — so a field only the model knows about, or only the UI
//! knows about, is a compile-time/unit-test error rather than a silent
//! mismatch.
//!
//! Pure data + pure lookups. No I/O, no DB, no `AppHandle`.

use crate::db::Database;
use crate::services::i18n::Language;
use serde::Serialize;

/// How one field is typed, and what the model is allowed to put in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FieldKind {
    /// Single-line free text.
    Text,
    /// Multi-line free text (a prompt, a description).
    LongText,
    /// A whole or decimal number.
    Number,
    /// A checkbox / switch.
    Bool,
    /// One of a fixed set. Anything else is dropped by the parser.
    #[serde(rename_all = "camelCase")]
    Enum { options: &'static [&'static str] },
    /// A list of free-text strings (domains, addresses).
    StringList,
    /// A list of enum values, each from `options`.
    #[serde(rename_all = "camelCase")]
    EnumList { options: &'static [&'static str] },
    /// A repeating group of sub-fields — the lens columns.
    #[serde(rename_all = "camelCase")]
    ObjectList { fields: &'static [FieldDef] },
}

/// One field of a form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldDef {
    /// Stable key. Matches the camelCase serde name of the backing Rust
    /// input struct, so a filled form deserializes straight into it.
    pub key: &'static str,
    /// What this field is, written for the model. Keep it one sentence —
    /// every word here is paid for on a form-fill turn.
    pub description: &'static str,
    pub kind: FieldKind,
    /// The form cannot be submitted without it. The parser still returns a
    /// partial fill when it is missing; it just names it in `missing_required`
    /// so the chat can say what is still needed.
    pub required: bool,
}

/// One fillable form.
///
/// Deliberately NOT `PartialEq`: the `available` gate is a function pointer,
/// and comparing those is meaningless (the same function can have different
/// addresses across codegen units). Forms are compared by `id`.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormDef {
    /// Stable id. This is what the query planner names in its verdict.
    pub id: &'static str,
    /// One line, shown to the *planner* so it can pick this form. This is the
    /// only part of a form that rides in the planner prompt on every turn, so
    /// it must stay short.
    pub summary: &'static str,
    /// Where the UI opens the form. Same wire shape as
    /// `help_docs::nav::NavTarget` (`view/<name>` or `settings/<tab>`) plus an
    /// optional `#<anchor>` naming the dialog inside it.
    pub target: &'static str,
    pub fields: &'static [FieldDef],
    /// Whether the feature this form belongs to is switched on right now.
    ///
    /// Mirrors `Tool::is_available`: a form whose feature is off must never be
    /// offered to the planner, because routing to it would spend a turn opening
    /// a view the user cannot even reach. Fails closed — a gate that cannot
    /// read the DB hides the form.
    #[serde(skip)]
    pub available: fn(&Database) -> bool,
    /// What the chat says once the form is filled and open, in `lang`. The
    /// fill is a draft the model wrote, so it must ask for a review and a save.
    #[serde(skip)]
    pub fill_reply: fn(Language) -> &'static str,
}

fn lens_fill_reply(lang: Language) -> &'static str {
    match lang {
        Language::En => "I've configured the Lens by filling in the form. There may be errors or incomplete parts: review it and save it so the Lens is created.",
        Language::Es => "He configurado la lente rellenando el formulario. Puede haber errores o partes incompletas: revísalo y guárdalo para que se cree la lente.",
        Language::Fr => "J'ai configuré la lentille en remplissant le formulaire. Il peut y avoir des erreurs ou des parties incomplètes : vérifiez-le et enregistrez-le pour créer la lentille.",
        Language::De => "Ich habe die Linse eingerichtet, indem ich das Formular ausgefüllt habe. Es kann Fehler oder unvollständige Teile geben: prüfe es und speichere es, damit die Linse erstellt wird.",
    }
}

const LENS_COLUMN_FIELDS: &[FieldDef] = &[
    FieldDef {
        key: "key",
        description: "snake_case identifier for this column, e.g. invoice_total",
        kind: FieldKind::Text,
        required: true,
    },
    FieldDef {
        key: "label",
        description: "Human-readable column header in the user's language",
        kind: FieldKind::Text,
        required: true,
    },
    FieldDef {
        key: "type",
        description: "Data type of the column",
        kind: FieldKind::Enum {
            options: LENS_COLUMN_TYPES,
        },
        required: true,
    },
    FieldDef {
        key: "description",
        description: "What to extract for this column, written as an instruction for the extraction model",
        kind: FieldKind::LongText,
        required: false,
    },
    FieldDef {
        key: "enumValues",
        description: "Allowed values. Only when type is enum",
        kind: FieldKind::StringList,
        required: false,
    },
    FieldDef {
        key: "required",
        description: "Whether the extraction must always produce a value for this column",
        kind: FieldKind::Bool,
        required: false,
    },
    FieldDef {
        key: "isUniqueKey",
        description: "Deduplicate rows by this column. At most one column may set it",
        kind: FieldKind::Bool,
        required: false,
    },
];

/// Mirrors `LensColumnType` in `models/lens.rs` (serde `snake_case`).
const LENS_COLUMN_TYPES: &[&str] = &[
    "string", "text", "number", "currency", "date", "boolean", "enum", "email", "url",
];

/// Mirrors the `MAILBOXES` constant in `src/components/Lenses/LensCreateModal.tsx`.
const LENS_MAILBOXES: &[&str] = &["inbox", "sent", "archive", "spam", "trash"];

/// Mirrors the `CATEGORIES` constant in `src/components/Lenses/LensCreateModal.tsx`.
const LENS_CATEGORIES: &[&str] = &["Primary", "Promotions", "Social", "Updates", "Forums"];

/// Mirrors `Direction` in `models/lens.rs` (serde `snake_case`).
const LENS_DIRECTIONS: &[&str] = &["inbound", "outbound", "either"];

/// Create Lens. The pilot form: it is the most structured one in the app
/// (nested column list, three enums, two string lists), so a filler that
/// handles it handles the flat settings forms for free.
///
/// Keys mirror `CreateLensInput` + `LensScope` in `models/lens.rs`, flattened
/// one level (`scope.*` → `scopeMailboxes`, …) because small models nest
/// unreliably. `forms::lens::to_create_input` re-nests them.
pub const LENS_CREATE: FormDef = FormDef {
    id: "lens.create",
    summary: "Create a Lens: a table that extracts structured columns from a set of emails",
    target: "view/lenses#create",
    // Lenses is an experimental feature, off by default. With it off the view
    // does not exist, so neither does this form.
    available: |db| db.is_lenses_enabled().unwrap_or(false),
    fill_reply: lens_fill_reply,
    fields: &[
        FieldDef {
            key: "name",
            description: "Short name for the lens, in the user's language",
            kind: FieldKind::Text,
            required: true,
        },
        FieldDef {
            key: "icon",
            description: "A single emoji representing the lens",
            kind: FieldKind::Text,
            required: false,
        },
        FieldDef {
            key: "scopeAccount",
            description:
                "Email address of the account the lens is for, when the request names one. Omit for all accounts",
            kind: FieldKind::Text,
            required: false,
        },
        FieldDef {
            key: "scopeMailboxes",
            description: "Which mailboxes the lens reads. Default inbox",
            kind: FieldKind::EnumList {
                options: LENS_MAILBOXES,
            },
            required: false,
        },
        FieldDef {
            key: "scopeCategories",
            description: "Restrict to these Gmail-style categories. Omit for all",
            kind: FieldKind::EnumList {
                options: LENS_CATEGORIES,
            },
            required: false,
        },
        FieldDef {
            key: "scopeDirection",
            description: "inbound = mail the user received, outbound = mail the user sent",
            kind: FieldKind::Enum {
                options: LENS_DIRECTIONS,
            },
            required: false,
        },
        FieldDef {
            key: "scopeSenderDomains",
            description: "Only emails from these sender domains, e.g. stripe.com",
            kind: FieldKind::StringList,
            required: false,
        },
        FieldDef {
            key: "scopeQuery",
            description: "Keyword filter applied to the subject",
            kind: FieldKind::Text,
            required: false,
        },
        FieldDef {
            key: "columns",
            description: "The columns to extract, one per piece of data the user asked for",
            kind: FieldKind::ObjectList {
                fields: LENS_COLUMN_FIELDS,
            },
            required: true,
        },
        FieldDef {
            key: "promptText",
            description: "Instruction for the extraction model, describing what each email contains",
            kind: FieldKind::LongText,
            required: true,
        },
    ],
};

/// Every form the chat can fill. Adding a form = one entry here.
pub const FORMS: &[FormDef] = &[LENS_CREATE];

/// Look a form up by the id the planner emitted, ignoring whether its feature
/// is enabled. `None` for anything unknown, so a hallucinated id can never
/// reach the filler.
pub fn lookup(id: &str) -> Option<&'static FormDef> {
    FORMS.iter().find(|f| f.id == id)
}

/// Look a form up and refuse it when its feature is switched off.
///
/// This is what the turn uses: the planner's verdict is only actionable if the
/// form's own view exists for this user.
pub fn lookup_available(db: &Database, id: &str) -> Option<&'static FormDef> {
    lookup(id).filter(|f| (f.available)(db))
}

/// The catalog the query planner sees: one `id — summary` line per ENABLED
/// form. This is the *only* forms text that rides in a prompt on every turn, so
/// it is deliberately tiny; it varies only with Settings, not per turn, so it
/// still sits in the planner's cached head.
///
/// With every form's feature switched off it says so in one line rather than
/// leaving an empty list under "Forms:" for the model to hallucinate into.
pub fn catalog(db: &Database) -> String {
    let lines: Vec<String> = FORMS
        .iter()
        .filter(|f| (f.available)(db))
        .map(|f| format!("- {}: {}", f.id, f.summary))
        .collect();
    if lines.is_empty() {
        return "  (none available — never answer with a form)".to_string();
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lens_form_can_name_the_account_it_is_for() {
        // "crea una lente para la cuenta X…" left the form on All accounts:
        // the form had no field the model could put the account in.
        let form = lookup("lens.create").expect("lens.create is registered");
        let field = form
            .fields
            .iter()
            .find(|f| f.key == "scopeAccount")
            .expect("scopeAccount field");
        assert!(!field.required, "no account named = all accounts");
        assert_eq!(field.kind, FieldKind::Text);
    }

    #[test]
    fn lookup_finds_a_known_form() {
        let form = lookup("lens.create").expect("lens.create is registered");
        assert_eq!(form.id, "lens.create");
    }

    #[test]
    fn lookup_rejects_an_unknown_id() {
        assert!(lookup("lens.destroy").is_none());
        assert!(lookup("").is_none());
    }

    #[test]
    fn every_form_id_is_unique() {
        let mut ids: Vec<&str> = FORMS.iter().map(|f| f.id).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before, "duplicate form id in FORMS");
    }

    #[test]
    fn every_field_key_is_unique_within_its_form() {
        for form in FORMS {
            let mut keys: Vec<&str> = form.fields.iter().map(|f| f.key).collect();
            let before = keys.len();
            keys.sort_unstable();
            keys.dedup();
            assert_eq!(keys.len(), before, "duplicate field key in form {}", form.id);
        }
    }

    #[test]
    fn every_form_target_is_a_parseable_nav_target() {
        use crate::services::help_docs::nav::parse_nav_target;
        for form in FORMS {
            let base = form.target.split('#').next().unwrap_or_default();
            assert!(
                parse_nav_target(base).is_some(),
                "form {} has an unroutable target {}",
                form.id,
                form.target
            );
        }
    }

    /// A DB with every fillable form's feature switched on.
    fn db_with_forms_enabled() -> Database {
        let db = Database::new_for_testing().expect("create test db");
        db.set_preference("lenses_enabled", "true").expect("enable lenses");
        db
    }

    #[test]
    fn the_catalog_is_one_line_per_enabled_form() {
        let db = db_with_forms_enabled();
        let catalog = catalog(&db);
        assert_eq!(catalog.lines().count(), FORMS.len());
        assert!(catalog.contains("lens.create"));
    }

    #[test]
    fn a_form_whose_feature_is_off_is_not_offered_to_the_planner() {
        // Lenses is off by default. Offering the form anyway would route a turn
        // to a view the user cannot even open.
        let db = Database::new_for_testing().expect("create test db");
        let catalog = catalog(&db);
        assert!(!catalog.contains("lens.create"), "catalog: {catalog}");
        assert!(catalog.contains("none available"));
    }

    #[test]
    fn lookup_available_refuses_a_form_whose_feature_is_off() {
        let db = Database::new_for_testing().expect("create test db");
        assert!(lookup_available(&db, "lens.create").is_none());
        // The un-gated lookup still finds it, so the parser can resolve the id
        // before the turn decides whether it is actionable.
        assert!(lookup("lens.create").is_some());
    }

    #[test]
    fn lookup_available_returns_a_form_whose_feature_is_on() {
        let db = db_with_forms_enabled();
        assert!(lookup_available(&db, "lens.create").is_some());
    }

    #[test]
    fn the_catalog_stays_small_enough_to_live_in_the_planner_prompt() {
        // The planner prompt is ~1.5k tokens and runs on every open question.
        // A catalog that grows past a few hundred characters per form belongs
        // behind a lookup, not in the prompt.
        let db = db_with_forms_enabled();
        assert!(
            catalog(&db).len() < 600,
            "forms catalog is {} chars — too big for the planner prompt",
            catalog(&db).len()
        );
    }

    #[test]
    fn lens_column_types_match_the_lens_model() {
        // `LensColumnType` serializes snake_case; a new variant there must show
        // up here or the model will never be told it exists.
        use crate::models::lens::LensColumnType;
        let serialized = |t: LensColumnType| {
            serde_json::to_value(t)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default()
        };
        let all = [
            LensColumnType::String,
            LensColumnType::Text,
            LensColumnType::Number,
            LensColumnType::Currency,
            LensColumnType::Date,
            LensColumnType::Boolean,
            LensColumnType::Enum,
            LensColumnType::Email,
            LensColumnType::Url,
        ];
        let expected: Vec<String> = all.into_iter().map(serialized).collect();
        assert_eq!(expected, LENS_COLUMN_TYPES);
    }

    #[test]
    fn lens_directions_match_the_lens_model() {
        use crate::models::lens::Direction;
        let serialized = |d: Direction| {
            serde_json::to_value(d)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default()
        };
        let expected: Vec<String> = [Direction::Inbound, Direction::Outbound, Direction::Either]
            .into_iter()
            .map(serialized)
            .collect();
        assert_eq!(expected, LENS_DIRECTIONS);
    }
}
