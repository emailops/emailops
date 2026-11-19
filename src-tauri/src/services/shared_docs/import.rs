//! Import spreadsheets (.xlsx, .xlsm) into EO Docs sheets: one
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

/// Pure: the tabs of a workbook with their values, laid out from A1 and cut to
/// [`MAX_IMPORT_ROWS`] x [`MAX_IMPORT_COLS`]. A formula whose result the file
/// does not store (written by a tool that never calculated it) is kept as its
/// formula text. Empty tabs are left out.
///
/// Only `.xlsx` (and `.xlsm`) is read, cell by cell, keeping just the cells
/// inside the caps. calamine reads an `.xls` or `.ods` into one dense range
/// spanning its first and last used cells when it opens the file, so a few
/// bytes naming a far cell would ask for gigabytes before any cap applies.
pub fn read_workbook(bytes: &[u8]) -> Result<Vec<ImportedTab>> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(AppError::InvalidInput("The file is too large to import".into()));
    }
    let mut book: calamine::Xlsx<_> = calamine::Xlsx::new(Cursor::new(bytes)).map_err(|_| {
        AppError::InvalidInput(
            "EO Docs imports spreadsheets saved as .xlsx: save the file as .xlsx and try again".into(),
        )
    })?;
    let mut tabs = Vec::new();
    for name in book.sheet_names() {
        let unreadable =
            |e: calamine::XlsxError| AppError::InvalidInput(format!("The sheet \"{name}\" could not be read: {e}"));
        let mut cells = match book.worksheet_cells_reader(&name) {
            Ok(cells) => cells,
            // A chart sheet has no cells to import.
            Err(calamine::XlsxError::NotAWorksheet(_)) => continue,
            Err(e) => return Err(unreadable(e)),
        };
        let mut rows: Vec<Vec<String>> = Vec::new();
        let (mut height, mut width) = (0, 0);
        while let Some(cell) = cells.next_cell_with_formula().map_err(unreadable)? {
            let mut text = cell_text(&Data::from(cell.value));
            if text.is_empty() {
                if let Some(f) = cell.formula.filter(|f| !f.is_empty()) {
                    text = format!("={f}");
                }
            }
            if text.is_empty() {
                continue;
            }
            // A used cell past the caps still stretches the tab up to them.
            let (r, c) = (cell.pos.0 as usize, cell.pos.1 as usize);
            height = height.max((r + 1).min(MAX_IMPORT_ROWS));
            width = width.max((c + 1).min(MAX_IMPORT_COLS));
            if r < MAX_IMPORT_ROWS && c < MAX_IMPORT_COLS {
                if rows.len() <= r {
                    rows.resize_with(r + 1, Vec::new);
                }
                if rows[r].len() <= c {
                    rows[r].resize(c + 1, String::new());
                }
                rows[r][c] = text;
            }
        }
        rows.resize_with(height, Vec::new);
        for row in &mut rows {
            row.resize(width, String::new());
        }
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

/// Extensions EO Docs can import, for the file dialog's filter.
pub const IMPORT_EXTENSIONS: &[&str] = &["docx", "xlsx", "xlsm"];

/// A file the user picked to import, ready for the webview: Word files are
/// converted there, spreadsheets come back to [`read_workbook`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PickedFile {
    pub filename: String,
    /// Standard base64.
    pub data: String,
}

/// How many times a read that timed out is tried again. A file kept in iCloud
/// Drive (or another cloud folder) with only a placeholder on disk is
/// downloaded on the first read; when that takes longer than the system
/// waits, the read fails with a timeout while the download carries on.
const READ_ATTEMPTS: usize = 3;

/// Pure over `read`: the file's bytes, trying again while it is still being
/// downloaded from the cloud, and saying so if it never arrives.
fn read_downloading(filename: &str, mut read: impl FnMut() -> std::io::Result<Vec<u8>>) -> Result<Vec<u8>> {
    for _ in 1..READ_ATTEMPTS {
        match read() {
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
            other => return other.map_err(|e| AppError::IoError(format!("Could not read {filename}: {e}"))),
        }
    }
    read().map_err(|e| match e.kind() {
        std::io::ErrorKind::TimedOut => AppError::FileNotDownloaded {
            filename: filename.to_string(),
        },
        _ => AppError::IoError(format!("Could not read {filename}: {e}")),
    })
}

/// Read the file the user picked in the native dialog. Only importable
/// extensions and sizes up to [`MAX_IMPORT_BYTES`] are read. Blocks, possibly
/// for a while on a file that is still in the cloud: call it off the async
/// runtime.
pub fn read_picked_file(path: &std::path::Path) -> Result<PickedFile> {
    use base64::Engine;
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| AppError::InvalidInput("The file has no usable name".into()))?
        .to_string();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if !IMPORT_EXTENSIONS.contains(&ext.as_str()) {
        return Err(AppError::InvalidInput(format!("EO Docs cannot import {filename}")));
    }
    let size = std::fs::metadata(path)
        .map_err(|e| AppError::IoError(format!("Could not read {filename}: {e}")))?
        .len();
    if size > MAX_IMPORT_BYTES as u64 {
        return Err(AppError::InvalidInput("The file is too large to import".into()));
    }
    let bytes = read_downloading(&filename, || std::fs::read(path))?;
    Ok(PickedFile {
        filename,
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use yrs::updates::decoder::Decode;
    use yrs::Out;

    const BUDGET: &[u8] = include_bytes!("../../../tests/fixtures/eodocs/budget.xlsx");

    fn timed_out() -> std::io::Error {
        std::io::Error::from(std::io::ErrorKind::TimedOut)
    }

    #[test]
    fn a_cloud_file_still_downloading_is_read_again_until_it_arrives() {
        let mut calls = 0;
        let bytes = read_downloading("plan.docx", || {
            calls += 1;
            if calls < READ_ATTEMPTS {
                Err(timed_out())
            } else {
                Ok(vec![1, 2])
            }
        })
        .unwrap();
        assert_eq!(bytes, vec![1, 2]);
        assert_eq!(calls, READ_ATTEMPTS);
    }

    #[test]
    fn a_cloud_file_that_never_arrives_says_it_is_not_downloaded() {
        let mut calls = 0;
        let err = read_downloading("plan.docx", || {
            calls += 1;
            Err(timed_out())
        })
        .unwrap_err();
        assert!(matches!(err, AppError::FileNotDownloaded { ref filename } if filename == "plan.docx"));
        assert_eq!(calls, READ_ATTEMPTS);
    }

    #[test]
    fn any_other_read_error_fails_at_once() {
        let mut calls = 0;
        let err = read_downloading("plan.docx", || {
            calls += 1;
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        })
        .unwrap_err();
        assert!(matches!(err, AppError::IoError(_)));
        assert_eq!(calls, 1);
    }

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

    /// A1, B2 (a formula never calculated), and values in the last row out to
    /// XFD1048576: a 2 KB file whose used range is the whole grid. calamine's
    /// range for it asked for ~17 billion cells and took the app down.
    const FAR_CELL: &[u8] = include_bytes!("../../../tests/fixtures/eodocs/far-cell.xlsx");

    #[test]
    fn a_far_cell_is_clipped_to_the_caps_without_building_the_whole_grid() {
        let tabs = read_workbook(FAR_CELL).unwrap();
        let rows = &tabs[0].rows;
        assert_eq!(rows.len(), MAX_IMPORT_ROWS);
        assert!(rows.iter().all(|row| row.len() == MAX_IMPORT_COLS));
        assert_eq!(rows[0][0], "kept");
        assert_eq!(rows[1][1], "=SUM(1,2)");
    }

    fn refusal(bytes: &[u8]) -> String {
        match read_workbook(bytes) {
            Err(AppError::InvalidInput(message)) => message,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    /// calamine reads an `.ods` or `.xls` whole when it opens the file, sized by
    /// its first and last used cells, so only `.xlsx` is imported.
    #[test]
    fn an_ods_workbook_is_refused_with_a_hint_to_save_it_as_xlsx() {
        let ods = include_bytes!("../../../tests/fixtures/eodocs/one-cell.ods");
        assert!(refusal(ods).contains(".xlsx"));
    }

    #[test]
    fn a_legacy_xls_workbook_is_refused_with_a_hint_to_save_it_as_xlsx() {
        let mut xls = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        xls.resize(4096, 0);
        assert!(refusal(&xls).contains(".xlsx"));
    }

    #[test]
    fn a_picked_xls_or_ods_file_is_refused() {
        let dir = std::env::temp_dir().join(format!("eodocs-pick-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["old.xls", "calc.ODS"] {
            let path = dir.join(name);
            std::fs::write(&path, b"cells").unwrap();
            assert!(read_picked_file(&path).is_err(), "{name}");
        }
        std::fs::remove_dir_all(dir).unwrap();
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

    #[test]
    fn a_picked_spreadsheet_is_read_and_other_files_are_refused() {
        use base64::Engine;
        let dir = std::env::temp_dir().join(format!("eodocs-pick-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let sheet = dir.join("Budget.XLSX");
        std::fs::write(&sheet, BUDGET).unwrap();
        let other = dir.join("notes.pdf");
        std::fs::write(&other, b"%PDF").unwrap();

        let picked = read_picked_file(&sheet).unwrap();
        assert_eq!(picked.filename, "Budget.XLSX");
        assert_eq!(
            base64::engine::general_purpose::STANDARD.decode(picked.data).unwrap(),
            BUDGET
        );
        assert!(read_picked_file(&other).is_err());
        assert!(read_picked_file(&dir.join("missing.xlsx")).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
