//! The CRDT under a shared document: Yjs updates (lib0 v1 encoding), handled
//! with `yrs`, the Rust port of Yjs, so the webview's `yjs` and this side read
//! and write the same bytes.
//!
//! A document's stored state is one merged update. Merging works on the
//! encoded updates rather than on a loaded `Doc`, so an update whose
//! dependencies have not arrived yet (a lost or reordered mail) is kept, not
//! dropped, and integrates once the missing piece comes in.

use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Doc, ReadTxn, StateVector, Transact, Update};

use crate::models::error::{AppError, Result};

fn invalid(what: &str) -> AppError {
    AppError::InvalidInput(format!("Not a valid shared document {what}"))
}

fn decode_sv(sv: &[u8]) -> Result<StateVector> {
    StateVector::decode_v1(sv).map_err(|_| invalid("state vector"))
}

/// The state of a new, empty document.
pub fn empty_state() -> Vec<u8> {
    Doc::new().transact().encode_state_as_update_v1(&StateVector::default())
}

/// Fail unless `update` decodes as a Yjs v1 update.
pub fn validate_update(update: &[u8]) -> Result<()> {
    Update::decode_v1(update).map(|_| ()).map_err(|_| invalid("update"))
}

/// `state` with `update` merged in. Idempotent: merging an update already in
/// `state` changes nothing.
pub fn merge(state: &[u8], update: &[u8]) -> Result<Vec<u8>> {
    yrs::merge_updates_v1([state, update]).map_err(|_| invalid("update"))
}

/// What `state` holds once loaded: the state vector of everything that
/// integrated, and whether some of it waits on changes not received yet.
pub struct Integrated {
    pub state_vector: Vec<u8>,
    pub has_missing: bool,
}

pub fn integrate(state: &[u8]) -> Result<Integrated> {
    let update = Update::decode_v1(state).map_err(|_| invalid("state"))?;
    let doc = Doc::new();
    let mut txn = doc.transact_mut();
    txn.apply_update(update).map_err(|_| invalid("state"))?;
    Ok(Integrated {
        state_vector: txn.state_vector().encode_v1(),
        has_missing: txn.has_missing_updates(),
    })
}

/// The part of `state` a peer at `state_vector` has not seen.
pub fn diff(state: &[u8], state_vector: &[u8]) -> Result<Vec<u8>> {
    decode_sv(state_vector)?;
    yrs::diff_updates_v1(state, state_vector).map_err(|_| invalid("state"))
}

/// The state vector of an empty document: a peer we know nothing about.
pub fn empty_state_vector() -> Vec<u8> {
    StateVector::default().encode_v1()
}

/// The newest state every one of `vectors` has seen (per client, the lowest
/// clock). A diff against it reaches every peer in one message. No vectors
/// means no peer: the empty vector.
pub fn min_state_vector(vectors: &[Vec<u8>]) -> Result<Vec<u8>> {
    let decoded = vectors.iter().map(|v| decode_sv(v)).collect::<Result<Vec<_>>>()?;
    let Some((first, rest)) = decoded.split_first() else {
        return Ok(empty_state_vector());
    };
    let min: StateVector = first
        .iter()
        .map(|(client, clock)| (*client, rest.iter().fold(*clock, |m, sv| m.min(sv.get(client)))))
        .filter(|(_, clock)| *clock > 0)
        .collect();
    Ok(min.encode_v1())
}

/// Whether a peer at `theirs` lacks something we have at `ours`.
pub fn lacks(ours: &[u8], theirs: &[u8]) -> Result<bool> {
    let ours = decode_sv(ours)?;
    let theirs = decode_sv(theirs)?;
    Ok(ours.iter().any(|(client, clock)| theirs.get(client) < *clock))
}

/// The words a document holds, for the search index: a document's text
/// (one line per block) or a sheet's cell values. Formatting is dropped.
pub fn plain_text(state: &[u8], kind: crate::models::shared_docs::DocKind) -> Result<String> {
    use yrs::types::text::YChange;
    use yrs::{Any, Map, Out, Text, XmlFragment, XmlOut};

    fn walk<T: ReadTxn>(nodes: yrs::types::xml::XmlNodes<'_, T>, txn: &T, out: &mut String) {
        for node in nodes {
            match node {
                XmlOut::Element(e) => {
                    walk(e.children(txn), txn, out);
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                }
                XmlOut::Fragment(f) => walk(f.children(txn), txn, out),
                XmlOut::Text(t) => {
                    for chunk in t.diff(txn, YChange::identity) {
                        if let Out::Any(Any::String(s)) = chunk.insert {
                            out.push_str(&s);
                        }
                    }
                }
            }
        }
    }

    let update = Update::decode_v1(state).map_err(|_| invalid("state"))?;
    let doc = Doc::new();
    doc.transact_mut().apply_update(update).map_err(|_| invalid("state"))?;
    let txn = doc.transact();
    let mut out = String::new();
    match kind {
        crate::models::shared_docs::DocKind::Doc => {
            if let Some(body) = txn.get_xml_fragment("body") {
                walk(body.children(&txn), &txn, &mut out);
            }
        }
        crate::models::shared_docs::DocKind::Sheet => {
            if let Some(cells) = txn.get_map("cells") {
                for (_, value) in cells.iter(&txn) {
                    if let Out::Any(Any::String(s)) = value {
                        out.push_str(&s);
                        out.push(' ');
                    }
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Build real Yjs updates the way the webview does, for tests.
    use yrs::updates::decoder::Decode;
    use yrs::{Doc, GetString, ReadTxn, Text, Transact, Update};

    /// A peer editing the document's `body` text.
    pub struct Peer {
        pub doc: Doc,
    }

    impl Peer {
        pub fn new(client_id: u64) -> Self {
            Self {
                doc: Doc::with_client_id(client_id),
            }
        }

        /// Insert `text` at `index` and return the update this produced.
        pub fn insert(&self, index: u32, text: &str) -> Vec<u8> {
            let body = self.doc.get_or_insert_text("body");
            let before = self.doc.transact().state_vector();
            {
                let mut txn = self.doc.transact_mut();
                body.insert(&mut txn, index, text);
            }
            self.doc.transact().encode_state_as_update_v1(&before)
        }

        /// Append a paragraph to the `body` XmlFragment, as the doc editor
        /// does, and return the update this produced.
        pub fn paragraph(&self, text: &str) -> Vec<u8> {
            use yrs::{XmlElementPrelim, XmlFragment, XmlTextPrelim};
            let body = self.doc.get_or_insert_xml_fragment("body");
            let before = self.doc.transact().state_vector();
            {
                let mut txn = self.doc.transact_mut();
                body.push_back(
                    &mut txn,
                    XmlElementPrelim::new("paragraph", [XmlTextPrelim::new(text).into()]),
                );
            }
            self.doc.transact().encode_state_as_update_v1(&before)
        }

        pub fn apply(&self, update: &[u8]) {
            let mut txn = self.doc.transact_mut();
            #[allow(clippy::expect_used)] // test helper: a bad update is a test bug
            txn.apply_update(Update::decode_v1(update).expect("valid update"))
                .expect("update applies");
        }

        pub fn text(&self) -> String {
            let body = self.doc.get_or_insert_text("body");
            body.get_string(&self.doc.transact())
        }
    }

    /// The `body` text a stored state renders to.
    pub fn text_of(state: &[u8]) -> String {
        let peer = Peer::new(999);
        peer.apply(state);
        peer.text()
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{text_of, Peer};
    use super::*;

    #[test]
    fn concurrent_edits_converge_whatever_the_merge_order() {
        let alice = Peer::new(1);
        let bob = Peer::new(2);
        let a = alice.insert(0, "Hello");
        let b = bob.insert(0, "World");

        let ab = merge(&merge(&empty_state(), &a).unwrap(), &b).unwrap();
        let ba = merge(&merge(&empty_state(), &b).unwrap(), &a).unwrap();

        assert_eq!(text_of(&ab), text_of(&ba));
        assert_eq!(text_of(&ab).len(), "HelloWorld".len());
    }

    #[test]
    fn a_merged_state_keeps_the_value_a_concurrent_write_replaced() {
        // The webview finds concurrent cell overwrites (`sheetConflicts.ts`)
        // by reading the replaced value back out of the stored state, so
        // merging must never collect it.
        use yrs::{Map, WriteTxn};
        let set = |client: u64, value: &str| {
            let doc = Doc::with_client_id(client);
            let mut txn = doc.transact_mut();
            txn.get_or_insert_map("cells").insert(&mut txn, "r:c", value);
            txn.encode_update_v1()
        };
        let state = merge(
            &merge(&empty_state(), &set(1, "first-value")).unwrap(),
            &set(2, "other-value"),
        )
        .unwrap();
        let state = merge(&state, &state).unwrap();

        let has = |needle: &str| state.windows(needle.len()).any(|w| w == needle.as_bytes());
        assert!(has("first-value") && has("other-value"));
    }

    #[test]
    fn merging_the_same_update_twice_changes_nothing() {
        let alice = Peer::new(1);
        let a = alice.insert(0, "Hello");
        let once = merge(&empty_state(), &a).unwrap();
        let twice = merge(&once, &a).unwrap();
        assert_eq!(text_of(&twice), "Hello");
    }

    #[test]
    fn an_update_whose_predecessor_is_missing_is_kept_until_it_arrives() {
        let alice = Peer::new(1);
        let first = alice.insert(0, "Hello");
        let second = alice.insert(5, " there");

        let gap = merge(&empty_state(), &second).unwrap();
        assert!(integrate(&gap).unwrap().has_missing);

        let healed = merge(&gap, &first).unwrap();
        assert!(!integrate(&healed).unwrap().has_missing);
        assert_eq!(text_of(&healed), "Hello there");
    }

    #[test]
    fn a_diff_carries_only_what_the_peer_has_not_seen() {
        let alice = Peer::new(1);
        let first = alice.insert(0, "Hello");
        let state = merge(&merge(&empty_state(), &first).unwrap(), &alice.insert(5, "!")).unwrap();

        let bob = Peer::new(2);
        bob.apply(&first);
        let bob_sv = bob.doc.transact().state_vector().encode_v1();
        bob.apply(&diff(&state, &bob_sv).unwrap());

        assert_eq!(bob.text(), "Hello!");
        assert!(diff(&state, &bob_sv).unwrap().len() < state.len());
    }

    #[test]
    fn the_min_vector_is_what_every_peer_has_seen() {
        let alice = Peer::new(1);
        let a1 = alice.insert(0, "ab");
        let a2 = alice.insert(2, "cd");
        let full = integrate(&merge(&merge(&empty_state(), &a1).unwrap(), &a2).unwrap())
            .unwrap()
            .state_vector;
        let half = integrate(&merge(&empty_state(), &a1).unwrap()).unwrap().state_vector;

        let min = min_state_vector(&[full.clone(), half.clone()]).unwrap();
        assert_eq!(min, half);
        assert_eq!(
            min_state_vector(&[full.clone(), empty_state_vector()]).unwrap(),
            empty_state_vector()
        );
        assert_eq!(min_state_vector(&[]).unwrap(), empty_state_vector());
        assert!(lacks(&full, &half).unwrap());
        assert!(!lacks(&half, &full).unwrap());
        assert!(!lacks(&full, &full).unwrap());
    }

    // Updates produced by the webview's `yjs` (13.6): a paragraph in the
    // `body` XmlFragment the doc editor binds to, and two sheet cells written
    // by two clients. This side must read them exactly.
    const YJS_PARAGRAPH_AND_CELL: &str =
        "AQQLAAcBBGJvZHkDCXBhcmFncmFwaAcACwAGBAALAQxIb2xhIMOxYW5kw7ooAQVjZWxscwMwOjABdwVUb3RhbAA=";
    const YJS_SECOND_CELL: &str = "AQEWACgBBWNlbGxzAzA6MQF3AjQyAA==";

    #[test]
    fn updates_written_by_yjs_merge_and_read_back_here() {
        use base64::Engine;
        use yrs::{GetString, Map, Out};
        let b64 = base64::engine::general_purpose::STANDARD;
        let first = b64.decode(YJS_PARAGRAPH_AND_CELL).unwrap();
        let second = b64.decode(YJS_SECOND_CELL).unwrap();

        let state = merge(&merge(&empty_state(), &second).unwrap(), &first).unwrap();
        assert!(!integrate(&state).unwrap().has_missing);

        let peer = test_support::Peer::new(999);
        peer.apply(&state);
        let txn = peer.doc.transact();
        let body = txn.get_xml_fragment("body").unwrap();
        assert_eq!(body.get_string(&txn), "<paragraph>Hola ñandú</paragraph>");
        let cells = txn.get_map("cells").unwrap();
        let cell = |k: &str| match cells.get(&txn, k) {
            Some(Out::Any(any)) => any.to_string(),
            other => format!("{other:?}"),
        };
        assert_eq!((cell("0:0"), cell("0:1")), ("Total".to_string(), "42".to_string()));
    }

    #[test]
    fn plain_text_reads_a_documents_paragraphs_and_a_sheets_cells() {
        use crate::models::shared_docs::DocKind;
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD;
        let state = merge(
            &b64.decode(YJS_PARAGRAPH_AND_CELL).unwrap(),
            &b64.decode(YJS_SECOND_CELL).unwrap(),
        )
        .unwrap();

        assert_eq!(plain_text(&state, DocKind::Doc).unwrap().trim(), "Hola ñandú");
        let cells = plain_text(&state, DocKind::Sheet).unwrap();
        assert!(cells.contains("Total") && cells.contains("42"), "{cells}");
        assert_eq!(plain_text(&empty_state(), DocKind::Doc).unwrap(), "");
    }

    #[test]
    fn garbage_is_refused_as_invalid_input() {
        assert!(validate_update(b"\xff\xff\xff").is_err());
        assert!(merge(&empty_state(), b"\xff\xff\xff").is_err());
        assert!(lacks(b"\xff\xff", &empty_state_vector()).is_err());
    }
}
