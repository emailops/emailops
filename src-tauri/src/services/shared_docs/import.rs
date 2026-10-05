//! Import spreadsheets (.xlsx, .xlsm, .xls, .ods) into EO Docs sheets: one
//! sheet per non-empty tab, cell values only. Word documents are converted in
//! the webview (mammoth → the editor's own schema) and arrive as an ordinary
//! edit; this module only handles the spreadsheet side, read with `calamine`
//! so the untrusted file is parsed here rather than in the webview.

use std::io::Cursor;

use calamine::{Data, Reader};
use yrs::{Array, Doc, Map, ReadTxn, StateVector, Transact};

use crate::models::error::{AppError, Result};

/// Largest file accepted for import.
pub const MAX_IMPORT_BYTES: usize = 20 * 1024 * 1024;
/// Rows and columns kept per tab; a sheet travels by email as a whole.
pub const MAX_IMPORT_ROWS: usize = 5_000;
pub const MAX_IMPORT_COLS: usize = 100;
/// An imported sheet still gets room to type, like a new one.
const MIN_ROWS: usize = 20;
const MIN_COLS: usize = 8;

/// One tab of a workbook: its name and its cell texts, row by row.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedTab {
    pub name: String,
    pub rows: Vec<Vec<String>>,
}

fn cell_text(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Int(i) => i.to_string(),
        Data::Float(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", *f as i64),
        Data::Float(f) => f.to_string(),
        Data::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Data::DateTime(dt) => match dt.as_datetime() {
            Some(d) if d.time() == chrono::NaiveTime::MIN => d.format("%Y-%m-%d").to_string(),
            Some(d) => d.format("%Y-%m-%d %H:%M").to_string(),
            None => dt.as_f64().to_string(),
        },
        Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("#{e:?}"),
    }
}

/// Pure: the tabs of a workbook with their values. A formula whose result the
/// file does not store (written by a tool that never calculated it) is kept as
/// its formula text. Empty tabs are left out.
pub fn read_workbook(bytes: &[u8]) -> Result<Vec<ImportedTab>> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(AppError::InvalidInput("The file is too large to import".into()));
    }
    let mut book = calamine::open_workbook_auto_from_rs(Cursor::new(bytes))
        .map_err(|_| AppError::InvalidInput("Not a spreadsheet EO Docs can import".into()))?;
    let mut tabs = Vec::new();
    for name in book.sheet_names() {
        let values = book
            .worksheet_range(&name)
            .map_err(|e| AppError::InvalidInput(format!("The sheet \"{name}\" could not be read: {e}")))?;
        // Formulas are optional: a file without them, or a format that does
        // not keep them, imports its values only.
        let formulas = book.worksheet_formula(&name).ok();
        let (height, width) = values.get_size();
        let (height, width) = (height.min(MAX_IMPORT_ROWS), width.min(MAX_IMPORT_COLS));
        let (row0, col0) = values.start().unwrap_or((0, 0));
        let mut rows = vec![vec![String::new(); width]; height];
        for (r, row) in rows.iter_mut().enumerate() {
            for (c, slot) in row.iter_mut().enumerate() {
                let at = (row0 + r as u32, col0 + c as u32);
                let mut text = values.get_value(at).map(cell_text).unwrap_or_default();
                if text.is_empty() {
                    if let Some(f) = formulas.as_ref().and_then(|f| f.get_value(at)) {
                        if !f.is_empty() {
                            text = format!("={f}");
                        }
                    }
                }
                *slot = text;
            }
        }
        // Rows and columns before the first used cell keep the tab's layout.
        let lead_rows = vec![vec![String::new(); width + col0 as usize]; row0 as usize];
        let rows: Vec<Vec<String>> = lead_rows
            .into_iter()
            .chain(rows.into_iter().map(|row| {
                let mut padded = vec![String::new(); col0 as usize];
                padded.extend(row);
                padded
            }))
            .take(MAX_IMPORT_ROWS)
            .map(|mut row| {
                row.truncate(MAX_IMPORT_COLS);
                row
            })
            .collect();
        if rows.iter().flatten().any(|v| !v.is_empty()) {
            tabs.push(ImportedTab { name, rows });
        }
    }
    Ok(tabs)
}

/// Pure: a sheet document (Yjs v1 update) laid out as the webview's sheet
/// model expects — `rows` / `cols` id lists and `cells` keyed `row:col` —
/// holding `rows`. `new_id` makes the row and column ids.
pub fn sheet_state(rows: &[Vec<String>], mut new_id: impl FnMut() -> String) -> Vec<u8> {
    let height = rows.len().max(MIN_ROWS);
    let width = rows.iter().map(Vec::len).max().unwrap_or(0).max(MIN_COLS);
    let doc = Doc::new();
    let row_ids: Vec<String> = (0..height).map(|_| new_id()).collect();
    let col_ids: Vec<String> = (0..width).map(|_| new_id()).collect();
    {
        let rows_ref = doc.get_or_insert_array("rows");
        let cols_ref = doc.get_or_insert_array("cols");
        let cells = doc.get_or_insert_map("cells");
        let mut txn = doc.transact_mut();
        for id in &row_ids {
            rows_ref.push_back(&mut txn, id.as_str());
        }
        for id in &col_ids {
            cols_ref.push_back(&mut txn, id.as_str());
        }
        for (r, row) in rows.iter().enumerate() {
            for (c, value) in row.iter().enumerate() {
                if !value.is_empty() {
                    cells.insert(&mut txn, format!("{}:{}", row_ids[r], col_ids[c]), value.as_str());
                }
            }
        }
    }
    let txn = doc.transact();
    txn.encode_state_as_update_v1(&StateVector::default())
}

/// Pure: the title of an imported tab — the file name without its
/// extension, plus the tab name when the workbook has several.
pub fn tab_title(filename: &str, tab: &str, tabs: usize) -> String {
    let stem = std::path::Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(filename)
        .trim();
    let stem = if stem.is_empty() { "Sheet" } else { stem };
    if tabs > 1 {
        format!("{stem} · {tab}")
    } else {
        stem.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yrs::updates::decoder::Decode;
    use yrs::Out;

    const BUDGET: &[u8] = include_bytes!("../../../tests/fixtures/eodocs/budget.xlsx");

    #[test]
    fn a_workbook_reads_as_one_tab_per_used_sheet_with_its_values() {
        let tabs = read_workbook(BUDGET).unwrap();
        let names: Vec<_> = tabs.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["Budget", "Notes"], "the empty tab is left out");
        let budget = &tabs[0].rows;
        assert_eq!(budget[0], vec!["Item", "Amount", "Paid", "Due"]);
        assert_eq!(budget[1], vec!["Flights", "420", "TRUE", "2026-11-03"]);
        assert_eq!(budget[2][1], "585.5");
        // Written by a tool that never calculated it: the formula is kept.
        assert_eq!(budget[3][..2], ["Total".to_string(), "=SUM(B2:B3)".to_string()]);
    }

    #[test]
    fn a_tab_keeps_its_position_on_the_grid() {
        let tabs = read_workbook(BUDGET).unwrap();
        let notes = &tabs[1].rows;
        assert_eq!(notes[1][1], "Bring the badge");
        assert_eq!(notes[0].len(), 2);
    }

    #[test]
    fn a_file_that_is_not_a_spreadsheet_is_refused() {
        assert!(read_workbook(b"PK\x03\x04 not really").is_err());
        assert!(read_workbook(&vec![0u8; MAX_IMPORT_BYTES + 1]).is_err());
    }

    #[test]
    fn the_sheet_state_is_the_layout_the_editor_reads() {
        let mut n = 0;
        let state = sheet_state(
            &[vec!["a".into(), String::new()], vec![String::new(), "b".into()]],
            || {
                n += 1;
                format!("id{n}")
            },
        );
        let doc = Doc::new();
        doc.transact_mut()
            .apply_update(yrs::Update::decode_v1(&state).unwrap())
            .unwrap();
        let txn = doc.transact();
        let rows = txn.get_array("rows").unwrap();
        let cols = txn.get_array("cols").unwrap();
        assert_eq!((rows.len(&txn), cols.len(&txn)), (MIN_ROWS as u32, MIN_COLS as u32));
        let id = |arr: &yrs::ArrayRef, i: u32| match arr.get(&txn, i) {
            Some(Out::Any(a)) => a.to_string(),
            other => panic!("{other:?}"),
        };
        let cells = txn.get_map("cells").unwrap();
        assert_eq!(cells.len(&txn), 2);
        let key = format!("{}:{}", id(&rows, 1), id(&cols, 1));
        assert_eq!(cells.get(&txn, &key).map(|v| v.to_string(&txn)), Some("b".to_string()));
    }

    #[test]
    fn titles_come_from_the_file_and_the_tab() {
        assert_eq!(tab_title("Q3 budget.xlsx", "Sheet1", 1), "Q3 budget");
        assert_eq!(tab_title("Q3 budget.xlsx", "Travel", 2), "Q3 budget · Travel");
        assert_eq!(tab_title(".xlsx", "A", 1), ".xlsx");
    }
}
