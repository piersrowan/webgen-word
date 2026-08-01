//! The document: an HTML file, and the scaffolding that keeps it a *document* rather than a page.

use crate::docstyle::{self, Base};
use crate::page::{PageSetup, PAGE_META};
use crate::sanitise;

/// Assemble a complete standalone HTML file.
///
/// The **doctype matters**. Until 0.3.0 the saved file was `document.documentElement.outerHTML` and
/// nothing else — and `outerHTML` does not include the doctype, so every document Word saved opened
/// in **quirks mode** in every browser afterwards. Measured, not theorised. A word processor whose
/// premise is "a standalone `.html` that any browser opens" cannot ship that.
pub fn wrap(title: &str, setup: PageSetup, base_css: &str, custom_css: &str, body_inner: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"{meta}\" content=\"{page}\">\n<title>{title}</title>\n\
         <style id=\"{base_id}\">\n{base}</style>\n\
         <style id=\"{custom_id}\">\n{custom}</style>\n</head>\n<body>\n{body}</body>\n</html>\n",
        meta = PAGE_META,
        page = escape_attr(&setup.to_meta()),
        title = escape(title),
        base_id = docstyle::STYLE_ID,
        base = base_css,
        custom_id = docstyle::CUSTOM_STYLE_ID,
        custom = custom_css,
        body = body_inner,
    )
}

/// A new, empty document.
pub fn blank(base: &Base, setup: PageSetup) -> String {
    wrap(
        "Untitled",
        setup,
        &docstyle::base_css(base, setup),
        &docstyle::custom_css(&Default::default()),
        "<h1>Untitled</h1>\n<p><br></p>\n",
    )
}

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Attribute values additionally need their quotes escaped, or the attribute ends early.
pub fn escape_attr(s: &str) -> String {
    escape(s).replace('"', "&quot;")
}

/// The page setup a document carries, if it carries one. Read from the source text before loading.
pub fn page_setup_of(html: &str) -> Option<PageSetup> {
    PageSetup::from_meta(&sanitise::find_meta(html, PAGE_META)?)
}

/// Fit an opened document with the scaffolding Word relies on, **in the source text, before it is
/// loaded**.
///
/// Doing this to the source rather than to the DOM afterwards is what stops an opened document
/// appearing unstyled and then visibly restyling itself a moment later. It is also why an opened
/// file gets the house style at all: until 0.3.0 `load_into` handed the file's own bytes straight to
/// WebKit, so a document written anywhere else rendered with browser defaults and printed at
/// whatever geometry the app happened to hold.
///
/// Three things are guaranteed afterwards: a doctype, a `<meta name="webgen-page">`, and the two
/// style blocks — base first so the document's own block can override it by document order.
pub fn prepare(html: &str, base_css: &str, setup: PageSetup) -> String {
    let mut out = html.to_string();

    // The base block: replace its contents if the document already has one, otherwise insert it.
    match style_block_span(&out, docstyle::STYLE_ID) {
        Some(span) => out.replace_range(span, &format!("\n{base_css}")),
        None => {
            let block = format!("<style id=\"{}\">\n{base_css}</style>\n", docstyle::STYLE_ID);
            out = insert_into_head(&out, &block, true);
        }
    }

    // A place for the document's own overrides, so the style panel always has somewhere to write.
    if style_block_span(&out, docstyle::CUSTOM_STYLE_ID).is_none() {
        let block = format!("<style id=\"{}\">\n</style>\n", docstyle::CUSTOM_STYLE_ID);
        out = insert_into_head(&out, &block, false);
    }

    // The page geometry, so the file describes the shape it was written for.
    if sanitise::find_meta(&out, PAGE_META).is_none() {
        let meta = format!(
            "<meta name=\"{PAGE_META}\" content=\"{}\">\n",
            escape_attr(&setup.to_meta())
        );
        out = insert_into_head(&out, &meta, true);
    }

    if !out.trim_start().to_ascii_lowercase().starts_with("<!doctype") {
        out = format!("<!doctype html>\n{out}");
    }
    out
}

/// The byte range of the *contents* of `<style id="…">`, if the document has one.
fn style_block_span(html: &str, id: &str) -> Option<std::ops::Range<usize>> {
    let lower = html.to_ascii_lowercase();
    let needle = format!("id=\"{id}\"");
    let alt = format!("id='{id}'");
    let at = lower.find(&needle).or_else(|| lower.find(&alt))?;
    // Walk back to the `<` that opens this tag and check it really is a <style>.
    let open = lower[..at].rfind('<')?;
    if !lower[open..].starts_with("<style") {
        return None;
    }
    let content_start = open + lower[open..].find('>')? + 1;
    let content_end = content_start + lower[content_start..].find("</style")?;
    Some(content_start..content_end)
}

/// Put a fragment inside `<head>` — at the front when `first`, otherwise just before `</head>`.
///
/// A document with no `<head>` at all gets one. Browsers synthesise one, but we cannot write into a
/// head that only exists after parsing.
fn insert_into_head(html: &str, fragment: &str, first: bool) -> String {
    let lower = html.to_ascii_lowercase();
    if let Some(open) = lower.find("<head") {
        if let Some(gt) = lower[open..].find('>') {
            let after_open = open + gt + 1;
            let at = if first {
                after_open
            } else {
                lower.find("</head").unwrap_or(after_open)
            };
            let mut out = String::with_capacity(html.len() + fragment.len() + 1);
            out.push_str(&html[..at]);
            out.push('\n');
            out.push_str(fragment);
            out.push_str(&html[at..]);
            return out;
        }
    }
    // No head. Put one after <html …> if there is one, else at the very top.
    let head = format!("<head>\n{fragment}</head>\n");
    if let Some(open) = lower.find("<html") {
        if let Some(gt) = lower[open..].find('>') {
            let at = open + gt + 1;
            return format!("{}\n{head}{}", &html[..at], &html[at..]);
        }
    }
    format!("{head}{html}")
}

/// Is this file something we should open at all? Cheap, and only advisory -- the file chooser
/// filters by extension, this catches "renamed a JPEG to .html".
///
/// It looks at the **whole** file rather than the first kilobyte. The 1 KB window was wrong: a
/// document with a long comment or a large `<head>` before `<body>` failed it and was silently
/// replaced with a blank page.
pub fn looks_like_html(bytes: &[u8]) -> bool {
    // Enough to be sure, bounded so a huge file is not lowercased in full.
    let window = &bytes[..bytes.len().min(64 * 1024)];
    let head = String::from_utf8_lossy(window).to_ascii_lowercase();
    head.contains("<html")
        || head.contains("<!doctype html")
        || head.contains("<body")
        || head.contains("<head")
        || head.contains("<p>")
        || head.contains("<div")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::Paper;

    fn base() -> Base {
        Base {
            font_family: "DejaVu Sans".into(),
            font_pt: 11,
            line_height_pct: 145,
            text: "#1a1a1a".into(),
            heading: "#111111".into(),
            link: "#1a5fb4".into(),
            rule: "#333333".into(),
            img_border_px: 0,
            img_border_colour: "#dddddd".into(),
            img_radius_px: 0,
            img_shadow: false,
        }
    }

    #[test]
    fn a_new_document_is_a_standards_mode_html_file() {
        let html = blank(&base(), PageSetup::default());
        assert!(html.starts_with("<!doctype html>"), "quirks mode is not an option:\n{html}");
        assert!(html.contains("<meta charset=\"utf-8\">"));
        assert!(html.contains("id=\"webgen-doc-style\""));
        assert!(html.contains("id=\"webgen-doc-custom\""));
    }

    #[test]
    fn a_documents_page_setup_survives_a_round_trip_through_the_file() {
        let setup = PageSetup { paper: Paper::A5, top: 15.0, right: 18.0, bottom: 15.0, left: 22.0 };
        let html = blank(&base(), setup);
        assert_eq!(page_setup_of(&html), Some(setup));
    }

    #[test]
    fn a_document_with_no_page_meta_reports_none_rather_than_a_guess() {
        assert_eq!(page_setup_of("<!doctype html><html><body>hi</body></html>"), None);
    }

    #[test]
    fn the_title_is_escaped_into_the_file() {
        let html = wrap("A <b>&amp; B</b>", PageSetup::default(), "", "", "");
        assert!(html.contains("<title>A &lt;b&gt;&amp;amp; B&lt;/b&gt;</title>"), "{html}");
    }

    #[test]
    fn a_long_head_no_longer_defeats_the_sniff_test() {
        // The 1 KB window used to reject this and silently blank the document.
        let padding = "<!-- ".to_string() + &"x".repeat(4000) + " -->";
        let doc = format!("{padding}\n<html><body><p>real</p></body></html>");
        assert!(looks_like_html(doc.as_bytes()));
    }

    #[test]
    fn preparing_a_foreign_document_gives_it_everything_word_needs() {
        let foreign = "<html><head><title>From elsewhere</title></head><body><p>hi</p></body></html>";
        let out = prepare(foreign, "body { color: red; }", PageSetup::default());
        assert!(out.starts_with("<!doctype html>"), "{out}");
        assert!(out.contains("id=\"webgen-doc-style\""), "{out}");
        assert!(out.contains("id=\"webgen-doc-custom\""), "{out}");
        assert!(out.contains("name=\"webgen-page\""), "{out}");
        assert!(out.contains("<title>From elsewhere</title>"), "its own title is kept:\n{out}");
        assert!(out.contains("<p>hi</p>"), "its own body is kept:\n{out}");
    }

    #[test]
    fn preparing_replaces_the_base_block_rather_than_stacking_another() {
        let once = prepare(&blank(&base(), PageSetup::default()), "body { color: red; }", PageSetup::default());
        let twice = prepare(&once, "body { color: blue; }", PageSetup::default());
        assert_eq!(twice.matches("id=\"webgen-doc-style\"").count(), 1, "{twice}");
        assert!(twice.contains("color: blue"), "{twice}");
        assert!(!twice.contains("color: red"), "the old base style is gone:\n{twice}");
    }

    #[test]
    fn a_documents_own_custom_block_is_not_disturbed() {
        let doc = "<html><head><style id=\"webgen-doc-custom\">\nh1 { color: #003366; }\n</style></head><body></body></html>";
        let out = prepare(doc, "", PageSetup::default());
        assert!(out.contains("h1 { color: #003366; }"), "{out}");
        assert_eq!(out.matches("id=\"webgen-doc-custom\"").count(), 1);
    }

    #[test]
    fn the_documents_own_page_setup_is_not_overwritten_by_the_default() {
        let a5 = PageSetup { paper: Paper::A5, top: 5.0, right: 5.0, bottom: 5.0, left: 5.0 };
        let doc = blank(&base(), a5);
        let out = prepare(&doc, "", PageSetup::default());
        assert_eq!(page_setup_of(&out), Some(a5), "the file's own geometry wins");
    }

    #[test]
    fn a_fragment_with_no_head_still_becomes_a_document() {
        let out = prepare("<p>just a paragraph</p>", "body{}", PageSetup::default());
        assert!(out.starts_with("<!doctype html>"), "{out}");
        assert!(out.contains("<head>"), "{out}");
        assert!(out.contains("<p>just a paragraph</p>"), "{out}");
    }

    #[test]
    fn something_that_is_not_a_document_is_still_refused() {
        assert!(!looks_like_html(&[0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10]));
        assert!(!looks_like_html(b""));
        assert!(!looks_like_html(b"just some prose in a text file"));
    }
}
