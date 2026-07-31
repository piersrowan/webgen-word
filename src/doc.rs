//! The document: an HTML file, and the scaffolding that keeps it a *document* rather than a page.

use crate::page::PageSetup;

/// The stylesheet the editor renders with and the file is saved with.
///
/// It is written into the saved file rather than kept in the app, so the document opens looking the
/// same in a browser, on another machine, or attached to an email. A word processor whose output
/// only looks right in itself is not much of a word processor.
///
/// `@page` is emitted for the benefit of OTHER renderers (a browser's own print, `weasyprint`).
/// **It has no effect on our print path** — see `page.rs`. Emitting it anyway costs nothing and
/// makes the file honest about its intended geometry.
pub fn stylesheet(setup: PageSetup) -> String {
    format!(
        "@page {{ size: {paper}; margin: {t}mm {r}mm {b}mm {l}mm; }}\n\
         html, body {{ margin: 0; padding: 0; }}\n\
         body {{\n  \
             font: 11pt/1.45 \"DejaVu Sans\", sans-serif;\n  \
             color: #1a1a1a;\n  \
             max-width: {w}mm;\n  \
             margin: 0 auto;\n  \
             padding: 8mm 0;\n\
         }}\n\
         h1 {{ font-size: 22pt; margin: 0 0 2mm; letter-spacing: -0.02em; }}\n\
         h2 {{ font-size: 13pt; margin: 7mm 0 2mm; border-bottom: 1pt solid #333;\n     \
              padding-bottom: 1.5mm; break-after: avoid; page-break-after: avoid; }}\n\
         h3 {{ font-size: 11.5pt; margin: 5mm 0 1mm; }}\n\
         p {{ margin: 0 0 3mm; }}\n\
         ul, ol {{ margin: 0 0 3mm 6mm; padding: 0; }}\n\
         li {{ margin: 0 0 1mm; }}\n\
         a {{ color: #1a5fb4; }}\n\
         img {{ max-width: 100%; }}\n\
         table {{ border-collapse: collapse; }}\n\
         td, th {{ border: 0.6pt solid #999; padding: 1.5mm 2.5mm; }}\n\
         /* Keep a block together across a page break where it would read badly split. */\n\
         li, tr, h1, h2, h3 {{ break-inside: avoid; page-break-inside: avoid; }}\n\
         .pagebreak {{ break-before: page; page-break-before: always; height: 0; }}\n",
        paper = match setup.paper {
            crate::page::Paper::A4 => "A4",
            crate::page::Paper::Letter => "Letter",
            crate::page::Paper::Legal => "Legal",
            crate::page::Paper::A5 => "A5",
        },
        t = setup.top,
        r = setup.right,
        b = setup.bottom,
        l = setup.left,
        w = setup.content_width_mm(),
    )
}

/// A new, empty document.
pub fn blank(setup: PageSetup) -> String {
    wrap("Untitled", &stylesheet(setup), "<h1>Untitled</h1>\n<p><br></p>\n")
}

/// Assemble a complete standalone HTML file.
pub fn wrap(title: &str, css: &str, body_inner: &str) -> String {
    format!(
        "<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>{}</title>\n\
         <style>\n{}</style>\n</head>\n<body>\n{}</body>\n</html>\n",
        escape(title),
        css,
        body_inner
    )
}

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Pull `<title>` out of a loaded file so the window can show a real name.
pub fn title_of(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let s = lower.find("<title>")? + 7;
    let e = lower[s..].find("</title>")? + s;
    let t = html[s..e].trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// Is this file something we should open at all? Cheap, and only advisory -- the file chooser
/// filters by extension, this catches "renamed a JPEG to .html".
pub fn looks_like_html(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]).to_ascii_lowercase();
    head.contains("<html") || head.contains("<!doctype html") || head.contains("<body")
}
