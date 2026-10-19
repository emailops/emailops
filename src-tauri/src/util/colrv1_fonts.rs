//! Keep COLRv1 color fonts away from the AppImage's bundled WebKitGTK.
//!
//! The Skia build inside the WebKitGTK we bundle in the AppImage aborts its
//! web process (a `ColorStop` out-of-range assertion in
//! `colrv1_configure_skpaint`) on the first glyph it draws from a COLRv1
//! font. Current Fedora ships Noto Color Emoji only in that format, so any
//! emoji blanks the window. The `.deb` uses the distro's WebKitGTK and is not
//! affected, which is why this only runs from an AppImage.
//!
//! The workaround is the one the reporter verified: a fontconfig file that
//! includes the system configuration and rejects the COLRv1 fonts, set
//! through `FONTCONFIG_FILE` before GTK or WebKit start, so the web process
//! inherits it. Emoji then fall back to a monochrome font instead of
//! crashing. A `FONTCONFIG_FILE` the user set is never overridden.

use std::ffi::OsStr;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Font file extensions fontconfig would load and this module inspects.
const FONT_EXTENSIONS: [&str; 3] = ["ttf", "otf", "ttc"];

/// How deep to walk a font directory. Distros nest one or two levels
/// (`/usr/share/fonts/<family>/<file>`); the cap only guards against a
/// symlink loop.
const MAX_SCAN_DEPTH: usize = 6;

/// Whether this launch should get the generated fontconfig file.
///
/// Only when running from an AppImage (`APPIMAGE` is set by its runtime) and
/// only when the user has not chosen their own `FONTCONFIG_FILE`.
pub fn should_override(appimage: Option<&OsStr>, fontconfig_file: Option<&OsStr>) -> bool {
    let set = |v: Option<&OsStr>| v.is_some_and(|s| !s.is_empty());
    set(appimage) && !set(fontconfig_file)
}

/// Whether an sfnt font (or any face of a collection) carries a COLR table
/// of version 1 or later.
///
/// Reads only the table directory and the two-byte COLR version, so a
/// multi-megabyte emoji font costs a few small reads. Anything that does not
/// parse as a font is reported as `false`: fontconfig skips it as well.
pub fn is_colrv1_font<R: Read + Seek>(font: &mut R) -> std::io::Result<bool> {
    let tag = read_tag(font, 0)?;
    if &tag != b"ttcf" {
        return face_has_colrv1(font, 0);
    }
    // TrueType collection: header tag, version, numFonts, then one offset
    // per face.
    let num_fonts = read_u32(font, 8)?;
    for i in 0..num_fonts {
        let offset = read_u32(font, 12 + 4 * u64::from(i))?;
        if face_has_colrv1(font, u64::from(offset))? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn face_has_colrv1<R: Read + Seek>(font: &mut R, face_offset: u64) -> std::io::Result<bool> {
    let num_tables = read_u16(font, face_offset + 4)?;
    for i in 0..u64::from(num_tables) {
        let record = face_offset + 12 + 16 * i;
        if &read_tag(font, record)? == b"COLR" {
            let table_offset = read_u32(font, record + 8)?;
            return Ok(read_u16(font, u64::from(table_offset))? >= 1);
        }
    }
    Ok(false)
}

fn read_at<R: Read + Seek, const N: usize>(font: &mut R, at: u64) -> std::io::Result<[u8; N]> {
    let mut buf = [0u8; N];
    font.seek(SeekFrom::Start(at))?;
    font.read_exact(&mut buf)?;
    Ok(buf)
}

fn read_tag<R: Read + Seek>(font: &mut R, at: u64) -> std::io::Result<[u8; 4]> {
    read_at::<R, 4>(font, at)
}

fn read_u16<R: Read + Seek>(font: &mut R, at: u64) -> std::io::Result<u16> {
    read_at::<R, 2>(font, at).map(u16::from_be_bytes)
}

fn read_u32<R: Read + Seek>(font: &mut R, at: u64) -> std::io::Result<u32> {
    read_at::<R, 4>(font, at).map(u32::from_be_bytes)
}

/// Every COLRv1 font file under `dirs`, sorted. Unreadable directories and
/// files are skipped: a font fontconfig cannot read cannot crash WebKit.
pub fn find_colrv1_fonts(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for dir in dirs {
        scan_dir(dir, 0, &mut found);
    }
    found.sort();
    found.dedup();
    found
}

fn scan_dir(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    if depth > MAX_SCAN_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir(&path, depth + 1, found);
            continue;
        }
        let is_font = path
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|ext| FONT_EXTENSIONS.iter().any(|f| ext.eq_ignore_ascii_case(f)));
        if !is_font {
            continue;
        }
        let colrv1 = std::fs::File::open(&path)
            .and_then(|mut f| is_colrv1_font(&mut f))
            .unwrap_or(false);
        if colrv1 {
            found.push(path);
        }
    }
}

/// The fontconfig file: the system configuration plus a reject rule per
/// font.
pub fn fonts_conf(rejected: &[PathBuf]) -> String {
    let mut globs = String::new();
    for path in rejected {
        globs.push_str("      <glob>");
        globs.push_str(&xml_escape(&path.to_string_lossy()));
        globs.push_str("</glob>\n");
    }
    format!(
        "<?xml version=\"1.0\"?>\n\
         <!DOCTYPE fontconfig SYSTEM \"urn:fontconfig:fonts.dtd\">\n\
         <!-- Generated by EmailOps for its AppImage: rejects COLRv1 color fonts,\n\
         \x20    which crash the bundled WebKitGTK. Regenerated on every launch. -->\n\
         <fontconfig>\n\
         \x20 <include ignore_missing=\"yes\">/etc/fonts/fonts.conf</include>\n\
         \x20 <selectfont>\n\
         \x20   <rejectfont>\n\
         {globs}\
         \x20   </rejectfont>\n\
         \x20 </selectfont>\n\
         </fontconfig>\n"
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Directories fontconfig reads by default on Linux distros.
#[cfg(target_os = "linux")]
fn default_font_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/usr/local/share/fonts"),
    ];
    if let Some(data) = dirs::data_dir() {
        dirs.push(data.join("fonts"));
    }
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".fonts"));
    }
    dirs
}

/// Apply the workaround for this process and every process it spawns.
///
/// Must run before Tauri builds its first window: the web process reads
/// `FONTCONFIG_FILE` when WebKit spawns it. Every failure leaves the
/// environment untouched and is logged; the app still starts, as it did
/// before the workaround existed.
#[cfg(target_os = "linux")]
pub fn apply_appimage_workaround() {
    if !should_override(
        std::env::var_os("APPIMAGE").as_deref(),
        std::env::var_os("FONTCONFIG_FILE").as_deref(),
    ) {
        return;
    }
    let rejected = find_colrv1_fonts(&default_font_dirs());
    if rejected.is_empty() {
        return;
    }
    let Some(cache) = dirs::cache_dir() else {
        eprintln!("[startup] Warning: no cache directory; COLRv1 fonts stay enabled");
        return;
    };
    let path = cache.join("emailops").join("fonts.conf");
    let written =
        std::fs::create_dir_all(cache.join("emailops")).and_then(|()| std::fs::write(&path, fonts_conf(&rejected)));
    match written {
        Ok(()) => {
            eprintln!(
                "[startup] AppImage: rejecting {} COLRv1 font(s) via {}",
                rejected.len(),
                path.display()
            );
            // Runs first thing in `run()`, before Tauri or any of our own
            // threads exist, so nothing can read the environment concurrently.
            std::env::set_var("FONTCONFIG_FILE", &path);
        }
        Err(e) => eprintln!("[startup] Warning: could not write {}: {e}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A minimal sfnt: header + one table record per `(tag, body)`, bodies
    /// appended after the directory.
    fn sfnt(tables: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut out = vec![0, 1, 0, 0];
        out.extend_from_slice(&(tables.len() as u16).to_be_bytes());
        out.extend_from_slice(&[0; 6]);
        let mut offset = 12 + 16 * tables.len();
        let mut bodies = Vec::new();
        for (tag, body) in tables {
            out.extend_from_slice(*tag);
            out.extend_from_slice(&[0; 4]);
            out.extend_from_slice(&(offset as u32).to_be_bytes());
            out.extend_from_slice(&(body.len() as u32).to_be_bytes());
            offset += body.len();
            bodies.extend_from_slice(body);
        }
        out.extend(bodies);
        out
    }

    fn colr(version: u16) -> Vec<u8> {
        let mut body = version.to_be_bytes().to_vec();
        body.extend_from_slice(&[0; 12]);
        body
    }

    fn check(bytes: Vec<u8>) -> bool {
        is_colrv1_font(&mut Cursor::new(bytes)).unwrap_or(false)
    }

    #[test]
    fn colr_version_1_is_detected() {
        assert!(check(sfnt(&[(b"cmap", vec![0; 8]), (b"COLR", colr(1))])));
    }

    #[test]
    fn colr_version_0_is_not_colrv1() {
        assert!(!check(sfnt(&[(b"COLR", colr(0)), (b"CPAL", vec![0; 8])])));
    }

    #[test]
    fn font_without_colr_is_not_colrv1() {
        assert!(!check(sfnt(&[(b"CBDT", vec![0; 8]), (b"glyf", vec![0; 8])])));
    }

    #[test]
    fn truncated_file_is_not_a_colrv1_font() {
        assert!(is_colrv1_font(&mut Cursor::new(vec![0, 1, 0])).is_err());
    }

    #[test]
    fn collection_with_a_colrv1_face_is_detected() {
        let plain = sfnt(&[(b"glyf", vec![0; 8])]);
        let color = sfnt(&[(b"COLR", colr(1))]);
        // ttcf header: tag, version, numFonts = 2, two offsets.
        let header_len = 12 + 8;
        let mut ttc = b"ttcf".to_vec();
        ttc.extend_from_slice(&[0, 1, 0, 0]);
        ttc.extend_from_slice(&2u32.to_be_bytes());
        ttc.extend_from_slice(&(header_len as u32).to_be_bytes());
        ttc.extend_from_slice(&((header_len + plain.len()) as u32).to_be_bytes());
        // Face-relative offsets inside `color` must become file-relative.
        let color = rebase(color, header_len + plain.len());
        ttc.extend(plain);
        ttc.extend(color);
        assert!(check(ttc));
    }

    /// Shift every table offset of a one-face sfnt by `by` bytes, as a
    /// collection stores them relative to the start of the file.
    fn rebase(mut face: Vec<u8>, by: usize) -> Vec<u8> {
        let num_tables = u16::from_be_bytes([face[4], face[5]]) as usize;
        for i in 0..num_tables {
            let at = 12 + 16 * i + 8;
            let old = u32::from_be_bytes([face[at], face[at + 1], face[at + 2], face[at + 3]]);
            face[at..at + 4].copy_from_slice(&(old + by as u32).to_be_bytes());
        }
        face
    }

    #[test]
    fn override_applies_only_inside_an_appimage() {
        assert!(should_override(Some(OsStr::new("/home/u/EmailOps.AppImage")), None));
        assert!(!should_override(None, None));
        assert!(!should_override(Some(OsStr::new("")), None));
    }

    #[test]
    fn user_fontconfig_file_is_respected() {
        assert!(!should_override(
            Some(OsStr::new("/home/u/EmailOps.AppImage")),
            Some(OsStr::new("/home/u/.config/fonts.conf"))
        ));
    }

    #[test]
    fn conf_includes_system_config_and_rejects_each_font() {
        let conf = fonts_conf(&[
            PathBuf::from("/usr/share/fonts/emoji/Color-COLRv1.ttf"),
            PathBuf::from("/home/u/.fonts/Other.otf"),
        ]);
        assert!(conf.contains("<include ignore_missing=\"yes\">/etc/fonts/fonts.conf</include>"));
        assert!(conf.contains("<glob>/usr/share/fonts/emoji/Color-COLRv1.ttf</glob>"));
        assert!(conf.contains("<glob>/home/u/.fonts/Other.otf</glob>"));
    }

    #[test]
    fn conf_escapes_xml_in_paths() {
        let conf = fonts_conf(&[PathBuf::from("/fonts/A&B <x>.ttf")]);
        assert!(conf.contains("<glob>/fonts/A&amp;B &lt;x&gt;.ttf</glob>"));
    }

    #[test]
    fn scan_finds_only_colrv1_fonts_in_nested_dirs() {
        let root = tempfile::tempdir().unwrap();
        let nested = root.path().join("family");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("color.ttf"), sfnt(&[(b"COLR", colr(1))])).unwrap();
        std::fs::write(nested.join("bitmap.ttf"), sfnt(&[(b"CBDT", vec![0; 8])])).unwrap();
        std::fs::write(nested.join("notes.txt"), sfnt(&[(b"COLR", colr(1))])).unwrap();
        std::fs::write(root.path().join("garbage.otf"), b"nope").unwrap();

        let found = find_colrv1_fonts(&[root.path().to_path_buf(), root.path().join("missing")]);

        assert_eq!(found, vec![nested.join("color.ttf")]);
    }
}
