//! Cutting a document down to a **document**.
//!
//! webgen-word is a word processor, not an HTML editor. The file it opens may have come from
//! anywhere — a web page saved to disk, an email attachment, an export from another tool — and the
//! only parts of it that belong in a document are text, structure, pictures, and style. Everything
//! else is cut.
//!
//! ## The ordering that matters
//!
//! This runs on the **source text, before `load_html`**. That is not a stylistic choice: a
//! `<script>` in a document handed to `WebView::load_html` has already run by the time any DOM-level
//! cleanup could reach it. Cleaning after loading would be cleaning up after the thing you were
//! trying to prevent. So the sanitiser is a text pass in Rust, and WebKit never sees the script at
//! all.
//!
//! It runs a **second** time on save, over the DOM read back out of the editor. That catches
//! anything that arrived by a route the first pass never saw — chiefly paste, which can carry a
//! whole subtree in from a browser.
//!
//! ## What survives
//!
//! The rule from Piers, 2026-08-01: *"JS (& any imported asset excluding CSS / Font [only if they
//! print]) must be cut out of the document."* So a saved document may not reach out to the network
//! or the filesystem for anything **except** stylesheets and fonts:
//!
//! | Thing | What happens |
//! |---|---|
//! | `<script>`, `on*=` handlers, `javascript:` URLs | removed |
//! | `<iframe> <object> <embed> <applet> <canvas> <svg> <audio> <video>` | removed with their contents |
//! | `<form> <input> <button> <select> <textarea>` | tag removed, **text kept** — a document is not a form, but its words are still words |
//! | `<base>`, `<meta http-equiv=refresh>` | removed — both re-point the document out from under itself |
//! | `<img src=…>` local or relative | **embedded** as a `data:` URI, so the file stays one file |
//! | `<img src=…>` remote | removed, and counted |
//! | `url(…)` in CSS, local | embedded |
//! | `url(…)` in CSS, remote | removed **unless** it is a font or a stylesheet |
//! | `<link rel=stylesheet>`, font links, `@import` | kept — the one exception, per the rule above |
//! | hyperlinks (`<a href="https://…">`) | kept: a link is a reference, not an imported asset |
//!
//! Nothing here is silent. Every cut is counted into a [`Report`] and the window shows it, because a
//! word processor that quietly deletes part of the document you just opened is worse than one that
//! refuses to open it.

use std::path::{Path, PathBuf};

/// What a pass removed, so the window can say so.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    /// `<script>` elements, `on*` handlers and `javascript:` URLs.
    pub scripts: usize,
    /// Embedded/active elements dropped with their contents.
    pub embeds: usize,
    /// Interactive controls unwrapped (tag dropped, text kept).
    pub controls: usize,
    /// References to the network that were cut.
    pub remote: usize,
    /// Pictures turned into `data:` URIs.
    pub embedded: usize,
    /// Pictures that could not be embedded and were dropped.
    pub images_dropped: usize,
    /// Pictures the document refers to that are not there. **Kept, not dropped**: the reference is
    /// how the picture comes back when the file is restored, and in a template it is the whole
    /// point — `front.png` is meant to be missing until somebody saves one over it.
    pub missing: usize,
}

/// Where a document's pictures go.
#[derive(Debug, Clone)]
pub enum AssetPolicy {
    /// Every local picture becomes a `data:` URI. One file, nothing beside it — the export form,
    /// and the only thing possible for a document that has never been saved.
    Embed,
    /// Pictures live in a folder beside the document. The working form: readable markup, real
    /// files with real names, and a template you can save over from Paint.
    Folder { dir: std::path::PathBuf, name: String },
    /// Leave local references exactly as they are; only say which ones are missing.
    ///
    /// This is the **load** policy. Opening a document must not rearrange somebody's files before
    /// they have asked for anything to be saved — the copying happens on save, where the user can
    /// see what they are agreeing to.
    Keep,
}

impl Report {
    /// Used by the tests to assert a clean document is left completely alone.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_empty(&self) -> bool {
        *self == Report::default()
    }

    /// True when something was *removed*, as opposed to merely embedded. Embedding a picture is
    /// housekeeping and needs no announcement; deleting one does.
    pub fn removed_anything(&self) -> bool {
        self.scripts > 0
            || self.embeds > 0
            || self.controls > 0
            || self.remote > 0
            || self.images_dropped > 0
            || self.missing > 0
    }

    /// A sentence for the banner. `None` when there is nothing worth saying.
    pub fn summary(&self) -> Option<String> {
        if !self.removed_anything() {
            return None;
        }
        let mut parts: Vec<String> = Vec::new();
        let plural = |n: usize, one: &str, many: &str| {
            if n == 1 {
                format!("{n} {one}")
            } else {
                format!("{n} {many}")
            }
        };
        if self.scripts > 0 {
            parts.push(plural(self.scripts, "script", "scripts"));
        }
        if self.embeds > 0 {
            parts.push(plural(self.embeds, "embedded object", "embedded objects"));
        }
        if self.controls > 0 {
            parts.push(plural(self.controls, "form control", "form controls"));
        }
        if self.remote > 0 {
            parts.push(plural(self.remote, "external reference", "external references"));
        }
        if self.images_dropped > 0 {
            parts.push(plural(self.images_dropped, "picture", "pictures"));
        }
        let removed = if parts.is_empty() {
            None
        } else {
            let list = match parts.len() {
                1 => parts[0].clone(),
                n => format!("{} and {}", parts[..n - 1].join(", "), parts[n - 1]),
            };
            Some(format!(
                "Removed {list} — this is a word processor, so documents keep text, pictures and style only."
            ))
        };
        // Missing pictures are a different kind of news: nothing was removed, something is absent.
        let absent = (self.missing > 0).then(|| {
            format!(
                "{} not found — the pictures folder may not have come with this document.",
                plural(self.missing, "picture is", "pictures are")
            )
        });
        Some(match (removed, absent) {
            (Some(r), Some(a)) => format!("{r}  {a}"),
            (Some(r), None) => r,
            (None, Some(a)) => a,
            (None, None) => return None,
        })
    }
}

/// Elements removed **with everything inside them**. Their contents are not document text.
const DROP_TREE: &[&str] = &[
    "script", "noscript", "iframe", "object", "applet", "frame", "frameset", "canvas", "svg",
    "audio", "video", "template", "map",
];

/// Void members of [`DROP_TREE`] — no end tag to search for, so they are simply skipped.
const DROP_VOID: &[&str] = &["embed", "input", "source", "track", "param", "base"];

/// Elements whose *tag* goes but whose *text* stays. A pasted form still contains sentences.
const UNWRAP: &[&str] = &["form", "button", "select", "option", "optgroup", "textarea", "label", "fieldset", "legend", "output", "datalist"];

/// HTML void elements — never expect an end tag for these.
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Elements whose content is raw text rather than markup, so `<` inside them starts nothing.
const RAW_TEXT: &[&str] = &["script", "style", "textarea", "title"];

/// Attributes that carry a URL, and so need their scheme checked.
const URL_ATTRS: &[&str] = &[
    "href", "src", "action", "formaction", "data", "poster", "background", "cite", "longdesc",
    "xlink:href", "ping", "srcdoc", "srcset",
];

/// Attributes dropped outright wherever they appear: they either import an asset we do not allow,
/// exist only to drive script — or, for `style`, violate the document's one styling rule.
///
/// **`style` is the no-inline invariant** (Piers, 2026-08-06): every visual fact lives in a
/// stylesheet — the document's own, or a table block's scoped one — where what was altered for
/// what outcome can actually be read. Inline styles make that unknowable, so they are not merely
/// discouraged, they are structurally impossible: anything a paste or a converter brings in is
/// stripped here, on load and again on save.
const DROP_ATTRS: &[&str] =
    &["srcdoc", "srcset", "ping", "formaction", "background", "longdesc", "style"];

/// The largest single asset that will be embedded. A document is meant to be mailable; a 50 MB
/// picture inlined as base64 becomes a 67 MB file that nothing will accept.
const MAX_EMBED_BYTES: u64 = 12 * 1024 * 1024;

/// Extensions we treat as fonts — allowed to stay as external references, per the rule.
const FONT_EXTS: &[&str] = &["woff", "woff2", "ttf", "otf", "eot", "sfnt"];

/// Clean a document. `base_dir` is the directory relative URLs resolve against — the document's own
/// folder when it came from disk, `None` for markup with no home (a paste).
pub fn clean(html: &str, base_dir: Option<&Path>, policy: &AssetPolicy) -> (String, Report) {
    let mut report = Report::default();
    let mut claimed: Vec<String> = Vec::new();
    let out = scan(html, base_dir, policy, &mut claimed, &mut report);
    (out, report)
}

/// Find `<meta name="…" content="…">` in source text, before it is ever loaded.
///
/// The page setup has to be read *before* the document is handed to WebKit, because the base
/// stylesheet is generated from it and is injected as part of the load. Going through the DOM would
/// mean styling the document a second time, visibly, after it was already on screen.
pub fn find_meta(html: &str, name: &str) -> Option<String> {
    let mut i = 0usize;
    while let Some(at) = html[i..].find('<').map(|d| i + d) {
        if let Some(tag) = parse_tag(html, at) {
            if tag.name == "meta"
                && !tag.is_end
                && tag.attr("name").map(|n| n.eq_ignore_ascii_case(name)).unwrap_or(false)
            {
                return tag.attr("content");
            }
            i = tag.end.max(at + 1);
        } else {
            i = at + 1;
        }
    }
    None
}

/// The tag scanner. Deliberately small and total: anything it does not understand is copied through
/// verbatim rather than guessed at, because a sanitiser that mangles valid markup is its own bug.
fn scan(
    html: &str,
    base_dir: Option<&Path>,
    policy: &AssetPolicy,
    claimed: &mut Vec<String>,
    report: &mut Report,
) -> String {
    let bytes = html.as_bytes();
    let mut out = String::with_capacity(html.len());
    let mut i = 0usize;
    // The element currently being dropped wholesale, and how deep we are inside same-named nesting.
    let mut dropping: Option<(String, usize)> = None;
    // Unwrapped elements whose end tag must also be swallowed, innermost last.
    let mut unwrapped: Vec<String> = Vec::new();

    while i < bytes.len() {
        if bytes[i] != b'<' {
            // Fast path: copy the run of text up to the next tag in one go.
            let next = html[i..].find('<').map(|d| i + d).unwrap_or(bytes.len());
            if dropping.is_none() {
                out.push_str(&html[i..next]);
            }
            i = next;
            continue;
        }

        // Comments and other bang-constructs (`<!doctype …`) are copied through untouched.
        if html[i..].starts_with("<!--") {
            let end = html[i..].find("-->").map(|d| i + d + 3).unwrap_or(bytes.len());
            if dropping.is_none() {
                out.push_str(&html[i..end]);
            }
            i = end;
            continue;
        }
        if html[i..].starts_with("<!") {
            let end = html[i..].find('>').map(|d| i + d + 1).unwrap_or(bytes.len());
            if dropping.is_none() {
                out.push_str(&html[i..end]);
            }
            i = end;
            continue;
        }

        let Some(tag) = parse_tag(html, i) else {
            // A bare `<` that starts no tag is just text.
            if dropping.is_none() {
                out.push('<');
            }
            i += 1;
            continue;
        };

        // --- inside a dropped subtree: track nesting, emit nothing ---------------------------
        if let Some((name, depth)) = dropping.as_mut() {
            if tag.name == *name {
                if tag.is_end {
                    *depth -= 1;
                    if *depth == 0 {
                        dropping = None;
                    }
                } else if !tag.self_closing {
                    *depth += 1;
                }
            }
            i = tag.end;
            // Raw-text elements inside a dropped subtree still hide their contents from the scanner.
            if !tag.is_end && RAW_TEXT.contains(&tag.name.as_str()) && !tag.self_closing {
                i = skip_raw_text(html, tag.end, &tag.name).1;
            }
            continue;
        }

        // --- end tags -------------------------------------------------------------------------
        if tag.is_end {
            if unwrapped.last().map(|n| n == &tag.name).unwrap_or(false) {
                unwrapped.pop();
            } else if !UNWRAP.contains(&tag.name.as_str()) {
                out.push_str(&html[i..tag.end]);
            }
            i = tag.end;
            continue;
        }

        // --- start tags -------------------------------------------------------------------------
        let name = tag.name.as_str();

        if DROP_VOID.contains(&name) {
            // `<base>` re-points every relative URL in the document; the rest are active content.
            if name == "base" {
                report.remote += 1;
            } else if name == "input" {
                report.controls += 1;
            } else {
                report.embeds += 1;
            }
            i = tag.end;
            continue;
        }

        if DROP_TREE.contains(&name) {
            if name == "script" {
                report.scripts += 1;
            } else {
                report.embeds += 1;
            }
            i = tag.end;
            if !tag.self_closing {
                if RAW_TEXT.contains(&name) {
                    i = skip_raw_text(html, tag.end, name).1;
                } else {
                    dropping = Some((name.to_string(), 1));
                }
            }
            continue;
        }

        if UNWRAP.contains(&name) {
            report.controls += 1;
            i = tag.end;
            if !tag.self_closing && !VOID.contains(&name) {
                unwrapped.push(name.to_string());
            }
            continue;
        }

        // `<meta http-equiv="refresh">` walks the document somewhere else on a timer.
        if name == "meta" {
            if tag
                .attr("http-equiv")
                .map(|v| v.trim().eq_ignore_ascii_case("refresh"))
                .unwrap_or(false)
            {
                report.remote += 1;
                i = tag.end;
                continue;
            }
        }

        // `<link>`: stylesheets and fonts stay, everything else (icons, preconnect, prefetch,
        // manifests) is an imported asset and goes.
        if name == "link" {
            let rel = tag.attr("rel").unwrap_or_default().to_ascii_lowercase();
            let href = tag.attr("href").unwrap_or_default();
            let keep = rel.split_whitespace().any(|t| t == "stylesheet")
                || is_font_url(&href)
                || rel.split_whitespace().any(|t| t == "preload")
                    && tag.attr("as").map(|a| a.eq_ignore_ascii_case("font")).unwrap_or(false);
            if !keep {
                report.remote += 1;
                i = tag.end;
                continue;
            }
        }

        // `<img>`: the one asset a document genuinely needs. Local ones become part of the file.
        if name == "img" {
            match place_image(&tag, base_dir, policy, claimed, report) {
                Some(rendered) => out.push_str(&rendered),
                None => {}
            }
            i = tag.end;
            continue;
        }

        out.push_str(&render_tag(&tag, report));
        i = tag.end;

        // Raw-text content: `<style>` is cleaned, `<title>`/`<textarea>` copied verbatim.
        if !tag.self_closing && RAW_TEXT.contains(&name) {
            let (text, next, closed) = skip_raw_text(html, tag.end, name);
            if name == "style" {
                out.push_str(&clean_css(text, base_dir, report));
            } else {
                out.push_str(text);
            }
            // The end tag was consumed along with the raw text, so put it back -- without this a
            // document's <title> swallowed the rest of its own head.
            if closed {
                out.push_str(&format!("</{name}>"));
            }
            i = next;
        }
    }

    out
}

/// One parsed start or end tag.
struct Tag {
    name: String,
    is_end: bool,
    self_closing: bool,
    /// `(name, value, quote)` — the quote character is kept so re-rendering does not change it.
    attrs: Vec<(String, String, char)>,
    /// Byte offset just past the closing `>`.
    end: usize,
}

impl Tag {
    fn attr(&self, name: &str) -> Option<String> {
        self.attrs
            .iter()
            .find(|(n, _, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v, _)| v.clone())
    }
}

/// Parse the tag starting at `start` (which must be a `<`). `None` if this is not a tag at all.
fn parse_tag(html: &str, start: usize) -> Option<Tag> {
    let bytes = html.as_bytes();
    let mut i = start + 1;
    let is_end = bytes.get(i) == Some(&b'/');
    if is_end {
        i += 1;
    }
    let name_start = i;
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b':') {
        i += 1;
    }
    if i == name_start {
        return None;
    }
    let name = html[name_start..i].to_ascii_lowercase();

    let mut attrs = Vec::new();
    let mut self_closing = false;
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        match bytes.get(i) {
            None => break,
            Some(b'>') => {
                i += 1;
                break;
            }
            Some(b'/') if bytes.get(i + 1) == Some(&b'>') => {
                self_closing = true;
                i += 2;
                break;
            }
            Some(b'/') => {
                i += 1;
                continue;
            }
            _ => {}
        }
        // Attribute name.
        let attr_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && bytes[i] != b'='
            && bytes[i] != b'>'
            && bytes[i] != b'/'
        {
            i += 1;
        }
        if i == attr_start {
            i += 1;
            continue;
        }
        let attr_name = html[attr_start..i].to_ascii_lowercase();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if bytes.get(i) != Some(&b'=') {
            attrs.push((attr_name, String::new(), '\0'));
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let (value, quote, next) = match bytes.get(i) {
            Some(&q @ (b'"' | b'\'')) => {
                let vs = i + 1;
                let end = html[vs..].find(q as char).map(|d| vs + d).unwrap_or(bytes.len());
                (html[vs..end].to_string(), q as char, (end + 1).min(bytes.len()))
            }
            _ => {
                let vs = i;
                let mut e = i;
                while e < bytes.len() && !bytes[e].is_ascii_whitespace() && bytes[e] != b'>' {
                    e += 1;
                }
                (html[vs..e].to_string(), '\0', e)
            }
        };
        i = next;
        attrs.push((attr_name, value, quote));
    }

    Some(Tag { name, is_end, self_closing, attrs, end: i })
}

/// The raw text of an element that has just been opened, the offset just past its end tag, and
/// whether that end tag was actually there.
fn skip_raw_text<'a>(html: &'a str, from: usize, name: &str) -> (&'a str, usize, bool) {
    let needle = format!("</{name}");
    let lower = html[from..].to_ascii_lowercase();
    match lower.find(&needle) {
        Some(d) => {
            let text_end = from + d;
            let close = html[text_end..].find('>').map(|e| text_end + e + 1).unwrap_or(html.len());
            (&html[from..text_end], close, true)
        }
        // Unterminated: the rest of the file is content, which is what a browser does too.
        None => (&html[from..], html.len(), false),
    }
}

/// Re-emit a start tag with its attributes filtered.
fn render_tag(tag: &Tag, report: &mut Report) -> String {
    let mut out = String::with_capacity(32);
    out.push('<');
    out.push_str(&tag.name);
    for (name, value, quote) in &tag.attrs {
        if name.starts_with("on") && name.len() > 2 {
            report.scripts += 1;
            continue;
        }
        if DROP_ATTRS.contains(&name.as_str()) {
            report.remote += 1;
            continue;
        }
        // `class=""` is what removing the last class leaves behind — noise in a document, and it
        // reaches the file every time an editing marker is stripped on the way out.
        if name == "class" && value.trim().is_empty() {
            continue;
        }
        if URL_ATTRS.contains(&name.as_str()) && is_script_url(value) {
            report.scripts += 1;
            continue;
        }
        out.push(' ');
        out.push_str(name);
        if *quote == '\0' && value.is_empty() {
            continue;
        }
        let q = if *quote == '\0' { '"' } else { *quote };
        out.push('=');
        out.push(q);
        out.push_str(value);
        out.push(q);
    }
    if tag.self_closing {
        out.push_str(" /");
    }
    out.push('>');
    out
}

/// The attribute carrying a picture's intended file name between being inserted and being written
/// out. A `data:` URI has no name of its own, and the name is the identity in the folder model.
pub const NAME_ATTR: &str = "data-wg-name";

/// Place an `<img>` according to the policy. `None` means the picture cannot be part of this
/// document at all and has to go.
fn place_image(
    tag: &Tag,
    base_dir: Option<&Path>,
    policy: &AssetPolicy,
    claimed: &mut Vec<String>,
    report: &mut Report,
) -> Option<String> {
    let src = tag.attr("src").unwrap_or_default();
    let src_trimmed = src.trim();
    let mut drop_name_attr = false;

    let new_src = if src_trimmed.is_empty() {
        report.images_dropped += 1;
        return None;
    } else if is_remote(src_trimmed) {
        // Still cut, under either policy: a document must not reach out to the network.
        report.remote += 1;
        report.images_dropped += 1;
        return None;
    } else {
        match policy {
            AssetPolicy::Keep => {
                if src_trimmed.starts_with("data:")
                    || resolve(src_trimmed, base_dir).map(|p| p.is_file()).unwrap_or(false)
                {
                    src.clone()
                } else {
                    report.missing += 1;
                    src.clone()
                }
            }
            AssetPolicy::Embed => {
                if src_trimmed.starts_with("data:") {
                    src.clone()
                } else {
                    match read_as_data_uri(src_trimmed, base_dir) {
                        Some(uri) => {
                            report.embedded += 1;
                            uri
                        }
                        None => {
                            // Nothing to embed. Keep the reference rather than deleting the
                            // picture: the file may come back.
                            report.missing += 1;
                            src.clone()
                        }
                    }
                }
            }
            AssetPolicy::Folder { dir, name } => {
                if let Some(rest) = src_trimmed.strip_prefix("data:") {
                    // A picture inserted into a document that had nowhere to put it yet. Now it
                    // has: write the bytes out under the name it came in with.
                    let wanted = tag
                        .attr(NAME_ATTR)
                        .filter(|n| !n.trim().is_empty())
                        .unwrap_or_else(|| {
                            let mime = rest.split([';', ',']).next().unwrap_or("");
                            format!("picture.{}", crate::assets::extension_for_mime(mime))
                        });
                    match write_data_uri(&src, &wanted, dir, claimed) {
                        Some(file) => {
                            drop_name_attr = true;
                            report.embedded += 1;
                            format!("{name}/{file}")
                        }
                        None => {
                            report.images_dropped += 1;
                            return None;
                        }
                    }
                } else if src_trimmed.starts_with(&format!("{name}/")) {
                    // Already where it belongs. Say so if the file is not actually there —
                    // a template's `front.png` is *meant* to be missing until one is saved over it.
                    if resolve(src_trimmed, base_dir).map(|p| p.is_file()).unwrap_or(false) {
                        src.clone()
                    } else {
                        report.missing += 1;
                        src.clone()
                    }
                } else {
                    // Somewhere else on disk: copy it in and point at the copy.
                    match copy_into(src_trimmed, base_dir, dir, claimed) {
                        Some(file) => {
                            report.embedded += 1;
                            format!("{name}/{file}")
                        }
                        None => {
                            report.missing += 1;
                            src.clone()
                        }
                    }
                }
            }
        }
    };

    // Rebuild with the new src, dropping the attributes the general filter would have dropped.
    let mut rebuilt = Tag {
        name: tag.name.clone(),
        is_end: false,
        self_closing: tag.self_closing,
        attrs: Vec::with_capacity(tag.attrs.len()),
        end: tag.end,
    };
    let mut seen_src = false;
    for (name, value, quote) in &tag.attrs {
        if drop_name_attr && name == NAME_ATTR {
            continue;
        }
        if name == "src" {
            seen_src = true;
            rebuilt.attrs.push((name.clone(), new_src.clone(), if *quote == '\0' { '"' } else { *quote }));
        } else {
            rebuilt.attrs.push((name.clone(), value.clone(), *quote));
        }
    }
    if !seen_src {
        rebuilt.attrs.push(("src".into(), new_src, '"'));
    }
    Some(render_tag(&rebuilt, report))
}

/// Write a `data:` URI out as a real file in the assets folder, and return the file name used.
fn write_data_uri(
    uri: &str,
    wanted: &str,
    dir: &Path,
    claimed: &mut Vec<String>,
) -> Option<String> {
    let (_, payload) = uri.split_once(',')?;
    let bytes = if uri[..uri.find(',')?].contains(";base64") {
        base64_decode(payload)?
    } else {
        percent_decode(payload).into_bytes()
    };
    let name = claim(wanted, dir, claimed);
    std::fs::create_dir_all(dir).ok()?;
    std::fs::write(dir.join(&name), bytes).ok()?;
    Some(name)
}

/// Copy a picture from wherever it is into the assets folder, and return the file name used.
fn copy_into(
    url: &str,
    base_dir: Option<&Path>,
    dir: &Path,
    claimed: &mut Vec<String>,
) -> Option<String> {
    let source = resolve(url, base_dir)?;
    if !source.is_file() {
        return None;
    }
    let wanted = source.file_name().map(|n| n.to_string_lossy().to_string())?;
    let name = claim(&wanted, dir, claimed);
    std::fs::create_dir_all(dir).ok()?;
    std::fs::copy(&source, dir.join(&name)).ok()?;
    Some(name)
}

/// Take a name in the folder, counting both what is on disk and what this pass has already used.
fn claim(wanted: &str, dir: &Path, claimed: &mut Vec<String>) -> String {
    let taken = |candidate: &str| claimed.iter().any(|c| c == candidate) || dir.join(candidate).exists();
    let name = crate::assets::unique_name(wanted, &taken);
    claimed.push(name.clone());
    name
}

/// The inverse of [`base64`], for turning an embedded picture back into a file.
fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for c in text.chars() {
        if c == '=' || c.is_whitespace() {
            continue;
        }
        let value = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' => 62,
            '/' => 63,
            _ => return None,
        };
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
}

/// Read a local file and turn it into a `data:` URI. `None` if it is missing, unreadable, or too
/// large to be worth carrying.
fn read_as_data_uri(url: &str, base_dir: Option<&Path>) -> Option<String> {
    let path = resolve(url, base_dir)?;
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_EMBED_BYTES {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    let mime = mime_for(&path);
    Some(format!("data:{mime};base64,{}", base64(&bytes)))
}

/// Turn a document-relative or `file://` URL into a path on disk.
fn resolve(url: &str, base_dir: Option<&Path>) -> Option<PathBuf> {
    let url = url.split(['?', '#']).next().unwrap_or(url);
    let decoded = percent_decode(url);
    if let Some(rest) = decoded.strip_prefix("file://") {
        return Some(PathBuf::from(rest));
    }
    let path = Path::new(&decoded);
    if path.is_absolute() {
        return Some(path.to_path_buf());
    }
    base_dir.map(|d| d.join(path))
}

/// `%20` and friends. Enough for file names; this is not a general URL parser.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "tif" | "tiff" => "image/tiff",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "css" => "text/css",
        _ => "application/octet-stream",
    }
}

/// Base64, written out rather than taken as a dependency: it is twenty lines and the OS build
/// vendors every crate we name.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { ALPHABET[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[n as usize & 63] as char } else { '=' });
    }
    out
}

fn is_script_url(value: &str) -> bool {
    let v: String = value.chars().filter(|c| !c.is_whitespace() && *c != '\0').collect();
    let v = v.to_ascii_lowercase();
    v.starts_with("javascript:") || v.starts_with("vbscript:") || v.starts_with("data:text/html")
}

fn is_remote(url: &str) -> bool {
    let u = url.trim().to_ascii_lowercase();
    u.starts_with("http://") || u.starts_with("https://") || u.starts_with("//") || u.starts_with("ftp://")
}

fn is_font_url(url: &str) -> bool {
    let u = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    FONT_EXTS.iter().any(|e| u.ends_with(&format!(".{e}")))
}

fn is_stylesheet_url(url: &str) -> bool {
    let u = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    u.ends_with(".css")
}

/// Clean the inside of a `<style>` element.
///
/// CSS is the one thing allowed to reach outside the file, so the work here is narrow: kill script
/// URLs, embed what is local, and cut remote references that are neither a font nor a stylesheet.
pub fn clean_css(css: &str, base_dir: Option<&Path>, report: &mut Report) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(at) = rest.find("url(") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 4..];
        let Some(close) = after.find(')') else {
            out.push_str(&rest[at..]);
            return out;
        };
        let raw = after[..close].trim();
        let url = raw.trim_matches(|c| c == '"' || c == '\'').trim();

        let replacement = if url.is_empty() || url.starts_with("data:") {
            format!("url({raw})")
        } else if is_script_url(url) {
            report.scripts += 1;
            "none".to_string()
        } else if is_remote(url) {
            if is_font_url(url) || is_stylesheet_url(url) {
                format!("url({raw})")
            } else {
                report.remote += 1;
                "none".to_string()
            }
        } else {
            match read_as_data_uri(url, base_dir) {
                Some(uri) => {
                    report.embedded += 1;
                    format!("url(\"{uri}\")")
                }
                None => {
                    report.remote += 1;
                    "none".to_string()
                }
            }
        };
        out.push_str(&replacement);
        rest = &after[close + 1..];
    }
    out.push_str(rest);

    // `expression(...)` is script in an old dialect; cheap to remove and there is no reason to keep it.
    if out.contains("expression(") {
        report.scripts += 1;
        out = out.replace("expression(", "none(");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean_str(html: &str) -> (String, Report) {
        clean(html, None, &AssetPolicy::Embed)
    }

    #[test]
    fn script_elements_go_with_their_contents() {
        let (out, r) = clean_str("<p>a</p><script>alert(1)</script><p>b</p>");
        assert_eq!(out, "<p>a</p><p>b</p>");
        assert_eq!(r.scripts, 1);
    }

    #[test]
    fn a_script_containing_markup_does_not_confuse_the_scanner() {
        // The classic sanitiser bug: `<` inside a script is not a tag.
        let (out, _) = clean_str("<script>if (a < b) { document.write('<p>x</p>') }</script><h1>t</h1>");
        assert_eq!(out, "<h1>t</h1>");
    }

    #[test]
    fn event_handlers_are_stripped_but_the_element_stays() {
        let (out, r) = clean_str(r#"<p onclick="steal()" class="keep">hello</p>"#);
        assert_eq!(out, r#"<p class="keep">hello</p>"#);
        assert_eq!(r.scripts, 1);
    }

    #[test]
    fn javascript_urls_lose_the_attribute_not_the_link_text() {
        let (out, r) = clean_str(r#"<a href="javascript:evil()">click</a>"#);
        assert_eq!(out, "<a>click</a>");
        assert_eq!(r.scripts, 1);
        // Whitespace and case are not a way past it.
        let (_, r2) = clean_str(r#"<a href=" JaVa\tScript:evil()">x</a>"#.replace("\\t", "\t").as_str());
        assert_eq!(r2.scripts, 1);
    }

    #[test]
    fn ordinary_hyperlinks_survive() {
        // A link is a reference, not an imported asset.
        let (out, r) = clean_str(r#"<a href="https://example.com/">docs</a>"#);
        assert_eq!(out, r#"<a href="https://example.com/">docs</a>"#);
        assert!(r.is_empty());
    }

    #[test]
    fn embedded_objects_go_with_their_contents() {
        let (out, r) = clean_str("<p>a</p><iframe src=\"https://x/\"><p>fallback</p></iframe><p>b</p>");
        assert_eq!(out, "<p>a</p><p>b</p>");
        assert_eq!(r.embeds, 1);
    }

    #[test]
    fn nested_same_name_drops_close_at_the_right_depth() {
        let (out, _) = clean_str("<svg><svg></svg></svg><p>after</p>");
        assert_eq!(out, "<p>after</p>");
    }

    #[test]
    fn form_controls_are_unwrapped_so_their_words_stay() {
        let (out, r) = clean_str("<form><label>Your name</label><input name=n><p>keep me</p></form>");
        assert_eq!(out, "Your name<p>keep me</p>");
        assert!(r.controls >= 1);
    }

    #[test]
    fn base_and_meta_refresh_are_removed() {
        let (out, r) = clean_str(r#"<head><base href="https://x/"><meta http-equiv="refresh" content="0;url=https://y/"><meta charset="utf-8"></head>"#);
        assert!(!out.contains("<base"));
        assert!(!out.contains("refresh"));
        assert!(out.contains(r#"<meta charset="utf-8">"#), "an ordinary meta survives: {out}");
        assert_eq!(r.remote, 2);
    }

    #[test]
    fn stylesheet_and_font_links_stay_but_icons_go() {
        let (out, r) = clean_str(
            r#"<link rel="stylesheet" href="a.css"><link rel="icon" href="f.png"><link rel="preload" as="font" href="x.woff2">"#,
        );
        assert!(out.contains(r#"rel="stylesheet""#));
        assert!(out.contains("x.woff2"));
        assert!(!out.contains("f.png"));
        assert_eq!(r.remote, 1);
    }

    #[test]
    fn remote_images_are_cut_rather_than_left_to_phone_home() {
        let (out, r) = clean_str(r#"<p><img src="https://tracker/pixel.gif" alt="x"></p>"#);
        assert_eq!(out, "<p></p>");
        assert_eq!(r.images_dropped, 1);
        assert_eq!(r.remote, 1);
    }

    #[test]
    fn a_data_uri_image_is_already_part_of_the_file_and_is_left_alone() {
        let src = "data:image/gif;base64,R0lGODlhAQABAAAAACw=";
        let (out, r) = clean_str(&format!(r#"<img src="{src}" width="10">"#));
        assert!(out.contains(src));
        assert!(out.contains(r#"width="10""#));
        assert_eq!(r.images_dropped, 0);
    }

    #[test]
    fn a_local_image_is_embedded_and_the_document_stops_depending_on_it() {
        let dir = std::env::temp_dir().join(format!("wgword-embed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A 1x1 GIF.
        let gif: [u8; 14] = [
            0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0x2c,
        ];
        std::fs::write(dir.join("pic.gif"), gif).unwrap();
        let (out, r) = clean(r#"<img src="pic.gif" alt="a">"#, Some(&dir), &AssetPolicy::Embed);
        assert!(out.contains("data:image/gif;base64,"), "not embedded: {out}");
        assert!(out.contains(r#"alt="a""#), "other attributes survive: {out}");
        assert_eq!(r.embedded, 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_local_image_is_reported_rather_than_deleted() {
        // Under the folder model a missing picture is REPORTED, not deleted: the reference is how
        // it comes back, and in a template `front.png` is meant to be absent until one is saved
        // over it.
        let (out, r) = clean(
            r#"<p><img src="gone.png"></p>"#,
            Some(Path::new("/nonexistent")),
            &AssetPolicy::Embed,
        );
        assert!(out.contains("gone.png"), "the reference survives: {out}");
        assert_eq!(r.missing, 1);
        assert_eq!(r.images_dropped, 0);
    }

    // ---- the folder policy -------------------------------------------------------------------

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("wgword-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn gif() -> Vec<u8> {
        vec![0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 1, 0, 1, 0, 0x80, 0, 0, 0x2c]
    }

    #[test]
    fn an_embedded_picture_becomes_a_real_file_with_the_name_it_came_in_with() {
        let dir = scratch("folder-name");
        let assets = dir.join("cats_files");
        let uri = format!("data:image/gif;base64,{}", base64(&gif()));
        let html = format!(r#"<img src="{uri}" {NAME_ATTR}="timmy.gif" alt="a">"#);
        let (out, r) = clean(
            &html,
            Some(&dir),
            &AssetPolicy::Folder { dir: assets.clone(), name: "cats_files".into() },
        );
        assert!(out.contains(r#"src="cats_files/timmy.gif""#), "{out}");
        assert!(!out.contains(NAME_ATTR), "the name attribute has done its job: {out}");
        assert!(out.contains(r#"alt="a""#), "other attributes survive: {out}");
        assert_eq!(std::fs::read(assets.join("timmy.gif")).unwrap(), gif());
        assert_eq!(r.embedded, 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_picture_from_elsewhere_is_copied_in_and_relinked() {
        let dir = scratch("folder-copy");
        let elsewhere = dir.join("Downloads");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("timmy.gif"), gif()).unwrap();
        let html = format!(r#"<img src="{}">"#, elsewhere.join("timmy.gif").display());
        let (out, _) = clean(
            &html,
            Some(&dir),
            &AssetPolicy::Folder { dir: dir.join("cats_files"), name: "cats_files".into() },
        );
        assert!(out.contains(r#"src="cats_files/timmy.gif""#), "{out}");
        assert!(dir.join("cats_files/timmy.gif").is_file());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn four_pictures_called_icon_end_up_as_four_files() {
        let dir = scratch("folder-collide");
        let uri = format!("data:image/gif;base64,{}", base64(&gif()));
        let one = format!(r#"<img src="{uri}" {NAME_ATTR}="icon.gif">"#);
        let html = one.repeat(4);
        let (out, _) = clean(
            &html,
            Some(&dir),
            &AssetPolicy::Folder { dir: dir.join("cats_files"), name: "cats_files".into() },
        );
        for name in ["icon.gif", "icon-2.gif", "icon-3.gif", "icon-4.gif"] {
            assert!(dir.join("cats_files").join(name).is_file(), "{name} missing");
            assert!(out.contains(&format!("cats_files/{name}")), "{out}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_picture_already_in_the_folder_is_left_where_it_is() {
        let dir = scratch("folder-keep");
        std::fs::create_dir_all(dir.join("cats_files")).unwrap();
        std::fs::write(dir.join("cats_files/front.png"), gif()).unwrap();
        let (out, r) = clean(
            r#"<img src="cats_files/front.png">"#,
            Some(&dir),
            &AssetPolicy::Folder { dir: dir.join("cats_files"), name: "cats_files".into() },
        );
        assert!(out.contains(r#"src="cats_files/front.png""#), "{out}");
        assert_eq!(r.embedded, 0, "nothing was copied or rewritten");
        assert_eq!(std::fs::read_dir(dir.join("cats_files")).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_templates_missing_placeholder_is_reported_and_kept() {
        // `front.png` is MEANT to be absent until somebody saves one over it from Paint. Deleting
        // the reference would break the template permanently.
        let dir = scratch("folder-missing");
        let (out, r) = clean(
            r#"<img src="cats_files/front.png">"#,
            Some(&dir),
            &AssetPolicy::Folder { dir: dir.join("cats_files"), name: "cats_files".into() },
        );
        assert!(out.contains("cats_files/front.png"), "{out}");
        assert_eq!(r.missing, 1);
        assert!(r.summary().unwrap().contains("not found"), "{:?}", r.summary());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn opening_a_document_moves_nobody_s_files() {
        let dir = scratch("keep-policy");
        std::fs::write(dir.join("timmy.gif"), gif()).unwrap();
        let (out, r) = clean(r#"<img src="timmy.gif">"#, Some(&dir), &AssetPolicy::Keep);
        assert!(out.contains(r#"src="timmy.gif""#), "left exactly as it was: {out}");
        assert!(r.is_empty());
        assert!(!dir.join("cats_files").exists(), "nothing was created on open");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn style_contents_survive_and_remote_backgrounds_do_not() {
        let (out, r) = clean_str(
            "<style>body { color: red; background: url(https://x/bg.png); }</style><p>t</p>",
        );
        assert!(out.contains("color: red"), "the CSS itself is kept: {out}");
        assert!(out.contains("background: none"), "{out}");
        assert_eq!(r.remote, 1);
    }

    #[test]
    fn remote_fonts_and_stylesheets_are_the_one_allowed_import() {
        let (out, r) = clean_str(
            "<style>@font-face { src: url(https://x/f.woff2); } @import url(https://x/a.css);</style>",
        );
        assert!(out.contains("f.woff2"), "{out}");
        assert!(out.contains("a.css"), "{out}");
        assert_eq!(r.remote, 0);
    }

    #[test]
    fn a_clean_document_is_returned_byte_for_byte() {
        // The commonest case by far: nothing to do, and nothing should be "tidied" either.
        let doc = "<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>CV</title>\n</head>\n<body>\n<h1>Name</h1>\n<p>A <b>bold</b> claim &amp; an entity.</p>\n<!-- a note -->\n</body>\n</html>\n";
        let (out, r) = clean_str(doc);
        assert_eq!(out, doc);
        assert!(r.is_empty());
    }

    #[test]
    fn an_empty_class_attribute_is_dropped() {
        // Removing the last class leaves `class=""`; it should not reach the saved file.
        let (out, _) = clean_str(r#"<p class="">text</p><p class=" ">more</p><p class="real">keep</p>"#);
        assert_eq!(out, r#"<p>text</p><p>more</p><p class="real">keep</p>"#);
    }

    #[test]
    fn unquoted_and_valueless_attributes_round_trip() {
        let (out, _) = clean_str("<td colspan=2 nowrap><img src=data:image/gif;base64,AA></td>");
        assert!(out.contains("colspan=\"2\""), "{out}");
        assert!(out.contains("nowrap"), "{out}");
    }

    #[test]
    fn inline_style_attributes_are_stripped_everywhere() {
        // The no-inline invariant: styling lives in sheets, full stop. Pasted markup, converter
        // output and hand-edited files all pass through here, so `style=` dies here.
        let (out, _) = clean_str(
            r#"<p style="color:red">a</p><table style="border:1px"><tr><td style="background:#eee" colspan="2">b</td></tr></table>"#,
        );
        assert!(!out.contains("style="), "{out}");
        assert!(out.contains("colspan=\"2\""), "the neighbours survive: {out}");
        // The sheet route stays open — that is the point.
        let (kept, _) = clean_str("<style>td { background: #eee; }</style><p>c</p>");
        assert!(kept.contains("background: #eee"), "{kept}");
    }

    #[test]
    fn the_summary_names_what_went() {
        let r = Report { scripts: 2, images_dropped: 1, ..Default::default() };
        let s = r.summary().unwrap();
        assert!(s.contains("2 scripts"), "{s}");
        assert!(s.contains("1 picture"), "{s}");
        assert!(Report::default().summary().is_none());
    }

    #[test]
    fn embedding_alone_is_not_worth_announcing() {
        let r = Report { embedded: 3, ..Default::default() };
        assert!(!r.removed_anything());
        assert!(r.summary().is_none());
    }

    #[test]
    fn base64_matches_the_known_answers() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn cleaning_twice_changes_nothing_the_second_time() {
        // Save runs the pass again over the DOM; it must be idempotent or every save would drift.
        let doc = r#"<p onclick="x()">a</p><script>y()</script><a href="https://e/">l</a>"#;
        let (once, _) = clean_str(doc);
        let (twice, r) = clean_str(&once);
        assert_eq!(once, twice);
        assert!(r.is_empty());
    }
}
