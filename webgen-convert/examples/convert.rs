//! Dev harness: `cargo run --example convert -- file.docx` writes `file.html` + `file_files/`
//! beside it. This is NOT the product path — webgen-word owns real conversions and runs the
//! result through its sanitiser; this exists to eyeball converter output during development.

fn main() {
    let arg = std::env::args().nth(1).expect("usage: convert <file.docx>");
    let path = std::path::Path::new(&arg);
    let bytes = std::fs::read(path).expect("read docx");
    let stem = path.file_stem().unwrap().to_string_lossy().to_string();
    let dir = format!("{stem}_files");
    let out = webgen_convert::docx_to_html(&bytes, &dir).expect("convert");

    let html = format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\"><title>{stem}</title></head>\n<body>\n{}\n</body></html>\n",
        out.body_html
    );
    let html_path = path.with_extension("html");
    std::fs::write(&html_path, html).expect("write html");
    if !out.assets.is_empty() {
        let adir = path.parent().unwrap().join(&dir);
        std::fs::create_dir_all(&adir).expect("mkdir assets");
        for (name, data) in &out.assets {
            std::fs::write(adir.join(name), data).expect("write asset");
        }
    }
    eprintln!(
        "{} -> {} ({} assets{})",
        path.display(),
        html_path.display(),
        out.assets.len(),
        if out.notes.is_empty() {
            String::new()
        } else {
            format!("; notes: {}", out.notes.join(" | "))
        }
    );
}
