//! Dev harness: docx -> our HTML -> docx. `cargo run --example roundtrip -- file.docx`
//! writes `file-roundtrip.docx` beside it. Not a product path; it exercises exactly what
//! Word's "Save a copy as Word" does, without a GUI.

fn main() {
    let arg = std::env::args().nth(1).expect("usage: roundtrip <file.docx>");
    let path = std::path::Path::new(&arg);
    let bytes = std::fs::read(path).expect("read docx");
    let stem = path.file_stem().unwrap().to_string_lossy().to_string();

    // In, exactly as Word does it.
    let out = webgen_convert::docx_to_segments(&bytes, &format!("{stem}_files")).expect("convert");
    let mut body = String::new();
    if !out.header_html.trim().is_empty() {
        body.push_str(&format!("<div class=\"wg-doc-header\">{}</div>\n", out.header_html));
    }
    for seg in out.segments {
        match seg {
            webgen_convert::Segment::Html(h) => body.push_str(&h),
            webgen_convert::Segment::Table(t) => body.push_str(&webgen_convert::render_table_html(&t)),
        }
    }
    if !out.footer_html.trim().is_empty() {
        body.push_str(&format!("<div class=\"wg-doc-footer\">{}</div>\n", out.footer_html));
    }
    let html = format!("<!doctype html><html><head><meta charset=\"utf-8\"></head><body>\n{body}\n</body></html>");

    // Out.
    let parsed = webgen_convert::from_html::parse(&html);
    let media: std::collections::HashMap<String, Vec<u8>> = out.assets.into_iter().collect();
    let page = out.page.map(|p| webgen_convert::to_docx::PageOut {
        width_mm: p.width_mm, height_mm: p.height_mm,
        top_mm: p.top_mm, right_mm: p.right_mm, bottom_mm: p.bottom_mm, left_mm: p.left_mm,
    }).unwrap_or(webgen_convert::to_docx::PageOut {
        width_mm: 210.0, height_mm: 297.0, top_mm: 20.0, right_mm: 20.0, bottom_mm: 20.0, left_mm: 20.0,
    });
    let docx = webgen_convert::to_docx::write_docx(&parsed.nodes, &media, page).expect("write docx");
    let target = path.with_file_name(format!("{stem}-roundtrip.docx"));
    std::fs::write(&target, docx).expect("write file");
    let tables = parsed.nodes.iter().filter(|n| matches!(n, webgen_convert::to_docx::Node::Table(_))).count();
    eprintln!("{} -> {} ({} blocks, {tables} tables, {} pictures)",
        path.display(), target.display(), parsed.nodes.len(), media.len());
}
