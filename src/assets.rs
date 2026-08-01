//! Where a document's pictures live.
//!
//! Until 0.7.0 every picture was embedded as a `data:` URI, on the grounds that one file is one
//! file: mail it, move it, nothing to lose. That is still true and still available — but it is the
//! wrong *working* format, for three reasons Piers set out and one from the code:
//!
//! - base64 is a third larger than the bytes it carries, and it makes the markup unreadable;
//! - a template wants **named** pictures. Put `front.png` and `back.png` in a brochure, then save
//!   over them from Paint without opening Word at all. That only works if the files are real files
//!   with the names you chose;
//! - opening the document in something else and saving it can mangle a very long line;
//! - and the undo fingerprint and the modified-check both serialise the body, so inlined pictures
//!   are re-hashed on every undo and every close.
//!
//! So the working format is **`cats.html` beside `cats_files/`**. Visible, not hidden — a hidden
//! folder cannot be dropped a new `front.png` without going hunting for it — and named after the
//! document rather than randomised, so which folder belongs to which document is obvious. The
//! document also says so itself, in `<meta name="webgen-assets">`, which is what lets Word tell you
//! *"3 pictures are missing"* instead of showing silent gaps.
//!
//! `<stem>_files` is not invented here: `webgen-browser`'s editor already writes exactly that, so a
//! document with pictures moves between the two intact.
//!
//! ## The one real cost, and what is done about it
//!
//! Two things now have to travel together, and the famous failure is that one of them does not.
//! Hence: Save As takes the folder with it, missing pictures are *reported*, and there are two
//! one-file formats a menu item away — `.wgz` (zipped) for sending, and a single-file export
//! (`data:` URIs) for pasting into an email.

use std::path::{Path, PathBuf};

/// The meta element naming a document's asset folder.
pub const META_ASSETS: &str = "webgen-assets";

/// The extension for the zipped one-file form.
pub const ZIP_EXTENSION: &str = "wgz";

/// `cats.html` → `cats_files`. The same convention the browser's editor uses.
pub fn folder_name(doc: &Path) -> String {
    let stem = doc.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "document".into());
    format!("{stem}_files")
}

/// The folder itself, beside the document.
pub fn folder_path(doc: &Path) -> PathBuf {
    doc.with_file_name(folder_name(doc))
}

/// A file name that is not taken yet: `icon.png`, then `icon-2.png`, then `icon-3.png`.
///
/// Names are kept, not replaced with `1.png`, `2.png` — an opaque name makes the template workflow
/// impossible, and the template workflow is the point of having a folder at all. `taken` is asked
/// rather than the directory listed, so the caller can also count names claimed earlier in the same
/// save that are not on disk yet.
pub fn unique_name(wanted: &str, taken: &dyn Fn(&str) -> bool) -> String {
    let wanted = sanitise_name(wanted);
    if !taken(&wanted) {
        return wanted;
    }
    let (stem, ext) = match wanted.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (wanted.clone(), String::new()),
    };
    for n in 2..10_000 {
        let candidate = format!("{stem}-{n}{ext}");
        if !taken(&candidate) {
            return candidate;
        }
    }
    wanted
}

/// Reduce a name to something safe to put on disk and in a URL.
///
/// A picture can come from anywhere, and "anywhere" includes names with slashes, colons and control
/// characters in them. The name is the identity here, so it has to be one a filesystem will accept.
pub fn sanitise_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches(['-', '.'].as_slice()).to_string();
    if trimmed.is_empty() {
        "picture.png".to_string()
    } else {
        trimmed
    }
}

/// A file extension for a `data:` URI's media type, so an embedded picture gets a sensible name
/// when it is written out.
pub fn extension_for_mime(mime: &str) -> &'static str {
    match mime.trim().to_ascii_lowercase().as_str() {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "image/bmp" => "bmp",
        "image/avif" => "avif",
        "image/x-icon" | "image/vnd.microsoft.icon" => "ico",
        "image/tiff" => "tiff",
        _ => "bin",
    }
}

// ---- the one-file form ---------------------------------------------------------------------
//
// A `.wgz` is an ordinary zip holding the document and its folder, so anything that opens zips can
// look inside one. Entries are STORED rather than deflated: pictures are already compressed, so
// deflate would buy a few percent on the markup and cost a compressor and a decompressor that the
// OS would have to vendor. A stored zip is still a zip.

/// Write a zip with stored (uncompressed) entries.
pub fn write_zip(path: &Path, entries: &[(String, Vec<u8>)]) -> std::io::Result<()> {
    let mut out: Vec<u8> = Vec::new();
    let mut directory: Vec<u8> = Vec::new();
    let mut count = 0u16;

    for (name, data) in entries {
        let offset = out.len() as u32;
        let crc = crc32(data);
        let name_bytes = name.as_bytes();

        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes()); // local file header
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&0u16.to_le_bytes()); // method: stored
        out.extend_from_slice(&0u16.to_le_bytes()); // time
        out.extend_from_slice(&0x0021u16.to_le_bytes()); // date: 1980-01-01
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra length
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(data);

        directory.extend_from_slice(&0x0201_4b50u32.to_le_bytes()); // central directory header
        directory.extend_from_slice(&20u16.to_le_bytes()); // version made by
        directory.extend_from_slice(&20u16.to_le_bytes()); // version needed
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0x0021u16.to_le_bytes());
        directory.extend_from_slice(&crc.to_le_bytes());
        directory.extend_from_slice(&(data.len() as u32).to_le_bytes());
        directory.extend_from_slice(&(data.len() as u32).to_le_bytes());
        directory.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes()); // extra
        directory.extend_from_slice(&0u16.to_le_bytes()); // comment
        directory.extend_from_slice(&0u16.to_le_bytes()); // disk
        directory.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        directory.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(name_bytes);
        count += 1;
    }

    let directory_offset = out.len() as u32;
    let directory_size = directory.len() as u32;
    out.extend_from_slice(&directory);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes()); // end of central directory
    out.extend_from_slice(&0u16.to_le_bytes()); // this disk
    out.extend_from_slice(&0u16.to_le_bytes()); // disk with directory
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&directory_size.to_le_bytes());
    out.extend_from_slice(&directory_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment length

    std::fs::write(path, out)
}

/// Pack a document and everything in its assets folder into a `.wgz`, and say how many entries
/// went in.
///
/// Takes markup that has already been through the sanitiser's folder policy, and the folder that
/// policy wrote into — so packing cannot drift from an ordinary save, because it *is* an ordinary
/// save followed by a zip.
pub fn pack(
    target: &Path,
    document_name: &str,
    html: &str,
    files_dir: &Path,
    folder: &str,
) -> std::io::Result<usize> {
    let mut entries: Vec<(String, Vec<u8>)> =
        vec![(document_name.to_string(), html.as_bytes().to_vec())];
    if let Ok(read) = std::fs::read_dir(files_dir) {
        let mut files: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
        files.sort();
        for file in files {
            if let (Some(name), Ok(bytes)) = (file.file_name(), std::fs::read(&file)) {
                entries.push((format!("{folder}/{}", name.to_string_lossy()), bytes));
            }
        }
    }
    let count = entries.len();
    write_zip(target, &entries)?;
    Ok(count)
}

/// What went wrong reading a `.wgz`, in words a person can act on.
#[derive(Debug, PartialEq, Eq)]
pub enum ZipError {
    NotAZip,
    Compressed,
    Damaged,
}

impl std::fmt::Display for ZipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ZipError::NotAZip => write!(f, "this file is not a .wgz"),
            ZipError::Compressed => write!(
                f,
                "this .wgz was re-packed with compression, which Word cannot read yet — unzip it and open the .html inside"
            ),
            ZipError::Damaged => write!(f, "this .wgz is damaged"),
        }
    }
}

/// Read a stored zip back.
pub fn read_zip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, ZipError> {
    // The end-of-central-directory record is at the end, after a comment of unknown length.
    let eocd = (0..bytes.len().saturating_sub(21))
        .rev()
        .find(|&i| bytes[i..].starts_with(&0x0605_4b50u32.to_le_bytes()))
        .ok_or(ZipError::NotAZip)?;
    let count = read_u16(bytes, eocd + 10)? as usize;
    let mut cursor = read_u32(bytes, eocd + 16)? as usize;

    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        if !bytes[cursor..].starts_with(&0x0201_4b50u32.to_le_bytes()) {
            return Err(ZipError::Damaged);
        }
        let method = read_u16(bytes, cursor + 10)?;
        if method != 0 {
            return Err(ZipError::Compressed);
        }
        let size = read_u32(bytes, cursor + 24)? as usize;
        let name_len = read_u16(bytes, cursor + 28)? as usize;
        let extra_len = read_u16(bytes, cursor + 30)? as usize;
        let comment_len = read_u16(bytes, cursor + 32)? as usize;
        let offset = read_u32(bytes, cursor + 42)? as usize;
        let name = String::from_utf8_lossy(
            bytes.get(cursor + 46..cursor + 46 + name_len).ok_or(ZipError::Damaged)?,
        )
        .to_string();

        // The local header repeats the name and extra fields, and its extra length may differ from
        // the central directory's — so the data offset has to be read from the local header.
        let local_name = read_u16(bytes, offset + 26)? as usize;
        let local_extra = read_u16(bytes, offset + 28)? as usize;
        let start = offset + 30 + local_name + local_extra;
        let data = bytes.get(start..start + size).ok_or(ZipError::Damaged)?.to_vec();

        entries.push((name, data));
        cursor += 46 + name_len + extra_len + comment_len;
    }
    Ok(entries)
}

fn read_u16(bytes: &[u8], at: usize) -> Result<u16, ZipError> {
    let slice = bytes.get(at..at + 2).ok_or(ZipError::Damaged)?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32, ZipError> {
    let slice = bytes.get(at..at + 4).ok_or(ZipError::Damaged)?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut table = [0u32; 256];
        for (n, slot) in table.iter_mut().enumerate() {
            let mut c = n as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *slot = c;
        }
        table
    });
    let mut crc = 0xFFFF_FFFFu32;
    for byte in data {
        crc = table[((crc ^ *byte as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folder_is_named_after_the_document_and_is_not_hidden() {
        assert_eq!(folder_name(Path::new("/home/p/cats.html")), "cats_files");
        assert_eq!(folder_path(Path::new("/home/p/cats.html")), Path::new("/home/p/cats_files"));
        // Not a dotfolder: a hidden one cannot be dropped a new front.png without going hunting.
        assert!(!folder_name(Path::new("cats.html")).starts_with('.'));
    }

    #[test]
    fn four_pictures_called_icon_do_not_collide() {
        let mut used: Vec<String> = Vec::new();
        for _ in 0..4 {
            let taken = |n: &str| used.iter().any(|u| u == n);
            let name = unique_name("icon.png", &taken);
            used.push(name);
        }
        assert_eq!(used, ["icon.png", "icon-2.png", "icon-3.png", "icon-4.png"]);
    }

    #[test]
    fn a_name_keeps_its_meaning_rather_than_becoming_a_number() {
        // The template workflow is the whole point of the folder: front.png has to stay front.png.
        let never = |_: &str| false;
        assert_eq!(unique_name("front.png", &never), "front.png");
        assert_eq!(unique_name("Timmy the Cat.jpeg", &never), "Timmy-the-Cat.jpeg");
    }

    #[test]
    fn a_name_from_anywhere_is_made_safe_for_a_filesystem() {
        assert_eq!(sanitise_name("/etc/passwd"), "passwd");
        assert_eq!(sanitise_name("../../escape.png"), "escape.png");
        assert_eq!(sanitise_name("a:b*c?.png"), "a-b-c-.png");
        assert_eq!(sanitise_name(""), "picture.png");
        assert_eq!(sanitise_name("..."), "picture.png");
    }

    #[test]
    fn a_data_uri_gets_a_sensible_extension() {
        assert_eq!(extension_for_mime("image/png"), "png");
        assert_eq!(extension_for_mime("image/JPEG"), "jpg");
        assert_eq!(extension_for_mime("application/octet-stream"), "bin");
    }

    #[test]
    fn a_zip_round_trips() {
        let entries = vec![
            ("cats.html".to_string(), b"<!doctype html><p>hi</p>".to_vec()),
            ("cats_files/front.png".to_string(), vec![0u8, 1, 2, 3, 255, 254]),
            ("cats_files/empty.bin".to_string(), Vec::new()),
        ];
        let dir = std::env::temp_dir().join(format!("wgword-zip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.wgz");
        write_zip(&path, &entries).unwrap();
        let read = read_zip(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(read, entries);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn packing_gathers_the_document_and_everything_beside_it() {
        let dir = std::env::temp_dir().join(format!("wgword-pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("cats_files")).unwrap();
        std::fs::write(dir.join("cats_files/front.png"), b"front").unwrap();
        std::fs::write(dir.join("cats_files/back.png"), b"back").unwrap();

        let target = dir.join("cats.wgz");
        let count = pack(
            &target,
            "cats.html",
            "<!doctype html>\n<p>hi</p>",
            &dir.join("cats_files"),
            "cats_files",
        )
        .unwrap();
        assert_eq!(count, 3, "the document and both pictures");

        let back = read_zip(&std::fs::read(&target).unwrap()).unwrap();
        let names: Vec<&str> = back.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["cats.html", "cats_files/back.png", "cats_files/front.png"]);
        assert_eq!(back[2].1, b"front");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_zip_we_wrote_is_a_zip_by_somebody_elses_reckoning() {
        // Round-tripping through our own reader would prove nothing if the writer were subtly
        // wrong in the same way. `unzip -t` is an independent opinion.
        let dir = std::env::temp_dir().join(format!("wgword-unzip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("check.wgz");
        write_zip(
            &path,
            &[
                ("cats.html".to_string(), b"<!doctype html>\n<p>hello</p>\n".to_vec()),
                ("cats_files/front.png".to_string(), (0u8..=255).collect()),
            ],
        )
        .unwrap();
        match std::process::Command::new("unzip").arg("-t").arg(&path).output() {
            Ok(out) => assert!(
                out.status.success(),
                "unzip -t rejected it:\n{}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
            // No unzip on the machine: the round-trip test still covers us.
            Err(_) => eprintln!("unzip not present, skipping the independent check"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_zip_says_what_is_wrong_rather_than_producing_rubbish() {
        assert_eq!(read_zip(b"not a zip at all").unwrap_err(), ZipError::NotAZip);
        assert_eq!(read_zip(&[]).unwrap_err(), ZipError::NotAZip);
    }

    #[test]
    fn the_checksum_matches_the_known_answer() {
        // If this is wrong every zip we write is subtly corrupt and only other tools would notice.
        assert_eq!(crc32(b""), 0x0000_0000);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b"The quick brown fox jumps over the lazy dog"), 0x414F_A339);
    }
}
