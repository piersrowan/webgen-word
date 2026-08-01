//! The document's style: a **base** stylesheet every document inherits, plus **per-document**
//! overrides that live in the document itself.
//!
//! ## Why two stylesheets
//!
//! A house style — the font, the colours, the page — belongs to the person, not to the file: set it
//! once and every new document has it. A particular document's departures from that — this report's
//! headings are navy, this CV's rules are hairlines — belong to the file, or they are lost the
//! moment it is opened on another machine.
//!
//! So: `<style id="webgen-doc-style">` carries the base, generated from settings
//! ([`crate::settings`] → the shared registry, CONTRACT.md §2), and `<style id="webgen-doc-custom">`
//! carries the document's own, written in a **fixed layout** so it can be read back as reliably as
//! it was written. We never parse arbitrary CSS — only the shape we emit. Anything else in the
//! document is left alone.
//!
//! ## This format is shared with the browser
//!
//! The element ids, the `wg-*` placement classes, the page-break class, [`STYLEABLE_TAGS`] and the
//! fixed layout below are **the same as `webgen-browser`'s `docstyle.rs`**, on the same terms as the
//! `assoc.rs`/`blockdev.rs` pair in CONTRACT.md §4/§4b: a document styled in the browser's editor
//! opens correctly styled in Word and back again. **Change the format here and it must change
//! there too** — see webgen-distro/CONTRACT.md §4c.

use std::collections::HashMap;

use webkit6::prelude::*;
use webkit6::WebView;

use crate::page::PageSetup;
use crate::settings::Settings;

/// The id of the base block — stable so re-injecting replaces rather than stacks.
pub const STYLE_ID: &str = "webgen-doc-style";

/// The id of the per-document block.
pub const CUSTOM_STYLE_ID: &str = "webgen-doc-custom";

/// Class marking a manual page break. `break-before` is what WebKit honours when printing; on screen
/// it shows as a dashed rule so the break is visible while editing.
pub const PAGE_BREAK_CLASS: &str = "webgen-page-break";

/// The class Word 0.1/0.2 used for the same thing. Still styled, so documents written then still
/// break where they were told to.
pub const LEGACY_PAGE_BREAK_CLASS: &str = "pagebreak";

/// The one shadow on offer, so a document looks of a piece rather than like five decisions.
pub const HOUSE_SHADOW: &str = "0 2px 10px rgba(0,0,0,.18)";

/// Everything the base stylesheet is built from. Read once so a dialog and the document cannot
/// disagree about what the current style is.
pub struct Base {
    pub font_family: String,
    pub font_pt: i64,
    pub line_height_pct: i64,
    pub text: String,
    pub heading: String,
    pub link: String,
    pub rule: String,
    pub img_border_px: i64,
    pub img_border_colour: String,
    pub img_radius_px: i64,
    pub img_shadow: bool,
}

impl Base {
    pub fn from_settings(settings: &Settings) -> Base {
        // One Pango string ("DejaVu Sans 11") rather than a family key and a size key, because that
        // is what the manifest's `font` row stores and what GTK's own picker returns.
        let desc = gtk::pango::FontDescription::from_string(&settings.string("doc_font", "DejaVu Sans 11"));
        let family = desc.family().map(|f| f.to_string()).unwrap_or_else(|| "DejaVu Sans".into());
        let pt = if desc.size() > 0 { (desc.size() / gtk::pango::SCALE) as i64 } else { 11 };
        Base {
            font_family: family,
            font_pt: pt.clamp(6, 48),
            line_height_pct: settings.i64("doc_line_height_pct", 145).clamp(100, 300),
            text: colour(settings, "doc_text_colour", "#1a1a1a"),
            heading: colour(settings, "doc_heading_colour", "#111111"),
            link: colour(settings, "doc_link_colour", "#1a5fb4"),
            rule: colour(settings, "doc_rule_colour", "#333333"),
            img_border_px: settings.i64("doc_img_border_px", 0).clamp(0, 20),
            img_border_colour: colour(settings, "doc_img_border_colour", "#dddddd"),
            img_radius_px: settings.i64("doc_img_radius_px", 0).clamp(0, 60),
            img_shadow: settings.bool("doc_img_shadow", false),
        }
    }
}

/// A stored colour, refused unless it is a bare `#rrggbb`. A colour knob is written into every
/// document, so a malformed one would be a stylesheet-wide syntax error rather than one odd row.
fn colour(settings: &Settings, key: &str, default: &str) -> String {
    let v = settings.string(key, default);
    let ok = v.len() == 7
        && v.starts_with('#')
        && v[1..].chars().all(|c| c.is_ascii_hexdigit());
    if ok {
        v
    } else {
        default.to_string()
    }
}

/// The base stylesheet: the house style, plus the page geometry this document is set to.
///
/// It is written into the saved file rather than kept in the app, so the document opens looking the
/// same in a browser, on another machine, or attached to an email. A word processor whose output
/// only looks right in itself is not much of a word processor.
///
/// `@page` is emitted for the benefit of OTHER renderers (a browser's own print, `weasyprint`).
/// **It has no effect on our print path** — see [`crate::page`]. Emitting it anyway costs nothing
/// and makes the file honest about its intended geometry.
pub fn base_css(base: &Base, setup: PageSetup) -> String {
    let shadow = if base.img_shadow { format!("box-shadow: {HOUSE_SHADOW};") } else { String::new() };
    let line = format!("{}.{:02}", base.line_height_pct / 100, base.line_height_pct % 100);
    format!(
        "/* Written by WebGen Word. The base style is in Settings > Word; this document's own
   departures from it are in the {CUSTOM_STYLE_ID} block below. */
@page {{ size: {paper}; margin: {t}mm {r}mm {b}mm {l}mm; }}
html, body {{ margin: 0; padding: 0; }}
body {{
  font: {pt}pt/{line} \"{family}\", \"DejaVu Sans\", sans-serif;
  color: {text};
  background: #ffffff;
  max-width: {w}mm;
  margin: 0 auto;
  padding: 8mm 0;
}}
h1, h2, h3, h4, h5, h6 {{ color: {heading}; }}
h1 {{ font-size: 2em; margin: 0 0 2mm; letter-spacing: -0.02em; }}
h2 {{ font-size: 1.2em; margin: 7mm 0 2mm; border-bottom: 1pt solid {rule};
     padding-bottom: 1.5mm; break-after: avoid; page-break-after: avoid; }}
h3 {{ font-size: 1.05em; margin: 5mm 0 1mm; }}
p {{ margin: 0 0 3mm; }}
/* Lists indent properly and nest visibly. The browser default (~4mm) is too tight to read as a
   list at print size, and nested levels were indistinguishable from their parent. */
ul, ol {{ margin: 0 0 3mm 0; padding-left: 9mm; }}
ul ul, ul ol, ol ul, ol ol {{ margin: 1mm 0 1mm 0; padding-left: 8mm; }}
li {{ margin: 0 0 1mm; }}
/* Distinct markers per level, so a sub-list reads as one at a glance. */
ul {{ list-style-type: disc; }}
ul ul {{ list-style-type: circle; }}
ul ul ul {{ list-style-type: square; }}
ol {{ list-style-type: decimal; }}
ol ol {{ list-style-type: lower-alpha; }}
ol ol ol {{ list-style-type: lower-roman; }}
a {{ color: {link}; }}
blockquote {{ margin: 3mm 0; padding: 0 0 0 4mm; border-left: 2pt solid {link}; }}
code, pre {{ font-family: ui-monospace, monospace; }}
table {{ border-collapse: collapse; }}
td, th {{ border: 0.6pt solid #999999; padding: 1.5mm 2.5mm; }}
img {{
  max-width: 100%; height: auto;
  border: {ib}px solid {ibc};
  border-radius: {ir}px;
  {shadow}
}}
/* Picture placement, set from the Picture menu. Classes rather than inline styles, so the document
   stays pure HTML plus stylesheets and a style change reaches every picture at once. */
.wg-left  {{ display: block; margin: 2mm auto 2mm 0; }}
.wg-right {{ display: block; margin: 2mm 0 2mm auto; }}
.wg-center{{ display: block; margin: 2mm auto; }}
.wg-wrap.wg-left  {{ float: left;  margin: 1mm 4mm 2mm 0; }}
.wg-wrap.wg-right {{ float: right; margin: 1mm 0 2mm 4mm; }}
.wg-clear {{ clear: both; }}
/* The picture the Picture menu is aimed at, and the element the style sidebar is aimed at.
   Editing only — both are stripped before saving. */
.wg-selected {{ outline: 3px solid {link}; outline-offset: 3px; }}
.wg-cursor {{ outline: 2px dashed {link}; outline-offset: 2px; }}
/* Keep a block together across a page break where it would read badly split. */
li, tr, h1, h2, h3 {{ break-inside: avoid; page-break-inside: avoid; }}
/* Manual page breaks: invisible in print, a dashed rule while editing. */
.{brk}, .{legacy} {{ break-before: page; page-break-before: always; border: 0; height: 0; }}
@media screen {{
  .{brk}, .{legacy} {{ height: auto; margin: 6mm 0; border-top: 2px dashed {link}; }}
}}
",
        paper = setup.paper.css_size(),
        t = setup.top as i64,
        r = setup.right as i64,
        b = setup.bottom as i64,
        l = setup.left as i64,
        pt = base.font_pt,
        family = base.font_family,
        text = base.text,
        heading = base.heading,
        link = base.link,
        rule = base.rule,
        w = setup.content_width_mm() as i64,
        ib = base.img_border_px,
        ibc = base.img_border_colour,
        ir = base.img_radius_px,
        brk = PAGE_BREAK_CLASS,
        legacy = LEGACY_PAGE_BREAK_CLASS,
    )
}

// ---- Per-document overrides ----------------------------------------------

/// Tags the per-document panel can style, in the order it lists them. Shared with the browser.
pub const STYLEABLE_TAGS: &[&str] = &[
    "h1", "h2", "h3", "h4", "h5", "h6", "p", "a", "ul", "ol", "li", "blockquote", "code", "pre",
    "table", "tr", "td", "th", "img", "figure", "figcaption", "hr", "div", "span", "section",
    "article",
];

/// The properties offered per tag. Kept deliberately short: this is document preparation, not a CSS
/// editor — every one of these is a thing a writer wants, and nothing here can break layout.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TagStyle {
    pub font_family: String,
    pub font_size: String,  // e.g. "1.2em" or "18px"
    /// `normal` | `bold`. Empty means "say nothing", which is not the same as `normal`: an explicit
    /// `normal` is how you take the bold off something that inherits it.
    pub font_weight: String,
    /// `normal` | `italic`.
    pub font_style: String,
    /// `none` | `underline` | `line-through`.
    pub text_decoration: String,
    pub colour: String,     // #rrggbb
    pub background: String, // #rrggbb
    pub border: String,     // e.g. "1px solid #cccccc"
    pub radius: String,     // e.g. "8px"
    /// Either empty or the house shadow. On/off rather than five more knobs.
    pub shadow: String,
    /// One value each, applied to all four sides — per-side control is a layout tool, and this is
    /// not that. e.g. "12px".
    pub padding: String,
    pub margin: String,
    /// Pictures and floated blocks: how the text sits around them.
    pub float: String,      // left | right | none
    pub text_align: String, // left | center | right | justify
}

impl TagStyle {
    pub fn is_empty(&self) -> bool {
        *self == TagStyle::default()
    }

    fn declarations(&self) -> Vec<(&'static str, &str)> {
        let mut out = Vec::new();
        for (prop, value) in [
            ("font-family", self.font_family.as_str()),
            ("font-size", self.font_size.as_str()),
            ("font-weight", self.font_weight.as_str()),
            ("font-style", self.font_style.as_str()),
            ("text-decoration", self.text_decoration.as_str()),
            ("color", self.colour.as_str()),
            ("background", self.background.as_str()),
            ("border", self.border.as_str()),
            ("border-radius", self.radius.as_str()),
            ("box-shadow", self.shadow.as_str()),
            ("padding", self.padding.as_str()),
            ("margin", self.margin.as_str()),
            ("float", self.float.as_str()),
            ("text-align", self.text_align.as_str()),
        ] {
            if !value.trim().is_empty() {
                out.push((prop, value.trim()));
            }
        }
        out
    }
}

/// Border styles offered by the picker. `none` is not among them: a border is removed by setting
/// its **width to 0**, which drops the declaration from the rule entirely rather than writing
/// `border: none` — Piers' rule, and the one that leaves the stanza clean.
pub const BORDER_STYLES: &[&str] = &["solid", "dashed", "dotted", "double", "groove", "ridge"];

/// Build a `border` declaration from the three pickers. **Width 0 means no declaration at all.**
pub fn compose_border(width_px: i64, style: &str, colour: &str) -> String {
    if width_px <= 0 {
        return String::new();
    }
    let style = if BORDER_STYLES.contains(&style) { style } else { "solid" };
    let colour = if colour.trim().is_empty() { "#000000" } else { colour.trim() };
    format!("{width_px}px {style} {colour}")
}

/// Read a `border` declaration back into the three pickers: `(width, style, colour)`.
///
/// Tolerant of what a person or another editor may have written — the parts are recognised by shape
/// and order does not matter. A width it cannot find is 0, which is the same as no border.
pub fn parse_border(value: &str) -> (i64, String, String) {
    let mut width = 0i64;
    let mut style = "solid".to_string();
    let mut colour = String::new();
    for part in value.split_whitespace() {
        let lower = part.to_ascii_lowercase();
        if let Some(n) = lower.strip_suffix("px").and_then(|n| n.parse::<f64>().ok()) {
            width = n.round() as i64;
        } else if BORDER_STYLES.contains(&lower.as_str()) {
            style = lower;
        } else if lower == "none" || lower == "hidden" {
            width = 0;
        } else if !lower.is_empty() {
            colour = named_colour(&lower).unwrap_or_else(|| part.trim().to_string());
        }
    }
    if colour.is_empty() {
        colour = "#000000".to_string();
    }
    (width.clamp(0, 40), style, colour)
}

/// The CSS colour names likely to appear in a border somebody typed by hand, so the picker opens on
/// the colour that is actually there rather than on black. Anything else is passed through as-is.
fn named_colour(name: &str) -> Option<String> {
    let hex = match name {
        "black" => "#000000",
        "white" => "#ffffff",
        "red" => "#ff0000",
        "green" => "#008000",
        "lime" => "#00ff00",
        "blue" => "#0000ff",
        "yellow" => "#ffff00",
        "orange" => "#ffa500",
        "purple" => "#800080",
        "grey" | "gray" => "#808080",
        "silver" => "#c0c0c0",
        "navy" => "#000080",
        "teal" => "#008080",
        "maroon" => "#800000",
        _ => return None,
    };
    Some(hex.to_string())
}

/// One document's overrides: **selector** → style.
///
/// Two kinds of key, and the distinction is the whole scoping model:
///
/// - a bare **tag** (`img`) — *all instances*. This is the default, and the common case: give
///   pictures a red border and every picture in the document has one.
/// - a **`.class`** or **`#id`** — *this instance*. One picture departs from the rule; it gets a
///   single-line override of its own and every other picture stays as it was.
///
/// Instance rules beat tag rules by CSS specificity, not by ordering, so an override is exactly the
/// properties it names — everything else still comes from the tag rule underneath it.
pub type CustomStyles = HashMap<String, TagStyle>;

/// The class Word mints when an element needs a handle of its own and has no `id` to use.
/// `wg-i1`, `wg-i2`, … — see [`next_instance_class`].
pub const INSTANCE_CLASS_PREFIX: &str = "wg-i";

/// Is `selector` one this format may carry? A styleable tag, or a single class or id.
///
/// Deliberately narrow. Descendant selectors, combinators and pseudo-classes are how a stylesheet
/// becomes something you cannot reason about from a panel, and the panel is the only writer here.
pub fn is_valid_selector(selector: &str) -> bool {
    let s = selector.trim();
    if STYLEABLE_TAGS.contains(&s) {
        return true;
    }
    let Some(name) = s.strip_prefix('.').or_else(|| s.strip_prefix('#')) else {
        return false;
    };
    !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// True for the keys that style one element rather than every element of a kind.
pub fn is_instance_selector(selector: &str) -> bool {
    selector.starts_with('.') || selector.starts_with('#')
}

/// The lowest `wg-iN` class not already used, given the highest already seen in the document.
pub fn next_instance_class(styles: &CustomStyles, seen_in_document: u32) -> String {
    let highest_in_css = styles
        .keys()
        .filter_map(|k| k.strip_prefix(&format!(".{INSTANCE_CLASS_PREFIX}")))
        .filter_map(|n| n.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    format!("{INSTANCE_CLASS_PREFIX}{}", highest_in_document(highest_in_css, seen_in_document) + 1)
}

fn highest_in_document(a: u32, b: u32) -> u32 {
    a.max(b)
}

/// Render the overrides in the fixed layout — one selector per line, properties in a fixed order.
///
/// Tag rules come first in [`STYLEABLE_TAGS`] order, then instance rules sorted by selector. The
/// order is for the diff and for the round-trip tests; the cascade does not depend on it.
pub fn custom_css(styles: &CustomStyles) -> String {
    let mut out = String::from("/* webgen-doc-custom v2 — written by the document style panel */\n");
    let rule = |selector: &str, style: &TagStyle| -> Option<String> {
        let declarations = style.declarations();
        if declarations.is_empty() {
            return None;
        }
        let body = declarations
            .iter()
            .map(|(p, v)| format!("{p}: {v};"))
            .collect::<Vec<_>>()
            .join(" ");
        Some(format!("{selector} {{ {body} }}\n"))
    };
    for tag in STYLEABLE_TAGS {
        if let Some(line) = styles.get(*tag).and_then(|s| rule(tag, s)) {
            out.push_str(&line);
        }
    }
    let mut instances: Vec<&String> = styles.keys().filter(|k| is_instance_selector(k)).collect();
    instances.sort_by_key(|s| instance_sort_key(s));
    for selector in instances {
        if let Some(line) = styles.get(selector).and_then(|s| rule(selector, s)) {
            out.push_str(&line);
        }
    }
    out
}

/// `.wg-i10` must sort after `.wg-i9`, so the minted ones sort numerically and everything else
/// falls in behind them alphabetically.
fn instance_sort_key(selector: &str) -> (u8, u32, String) {
    match selector
        .strip_prefix(&format!(".{INSTANCE_CLASS_PREFIX}"))
        .and_then(|n| n.parse::<u32>().ok())
    {
        Some(n) => (0, n, String::new()),
        None => (1, 0, selector.to_string()),
    }
}

/// Read the fixed layout back. Anything that is not our shape is ignored rather than guessed at.
pub fn parse_custom_css(css: &str) -> CustomStyles {
    let mut styles = CustomStyles::new();
    for line in css.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("/*") {
            continue;
        }
        let Some((tag, rest)) = line.split_once('{') else { continue };
        let tag = tag.trim();
        if !is_valid_selector(tag) {
            continue;
        }
        let Some(body) = rest.strip_suffix('}').map(str::trim) else { continue };
        let mut style = TagStyle::default();
        for declaration in body.split(';') {
            let Some((property, value)) = declaration.split_once(':') else { continue };
            let value = value.trim().to_string();
            match property.trim() {
                "font-family" => style.font_family = value,
                "font-size" => style.font_size = value,
                "font-weight" => style.font_weight = value,
                "font-style" => style.font_style = value,
                "text-decoration" => style.text_decoration = value,
                "color" => style.colour = value,
                "background" => style.background = value,
                "border" => style.border = value,
                "border-radius" => style.radius = value,
                "box-shadow" => style.shadow = value,
                "padding" => style.padding = value,
                "margin" => style.margin = value,
                "float" => style.float = value,
                "text-align" => style.text_align = value,
                _ => {}
            }
        }
        if !style.is_empty() {
            styles.insert(tag.to_string(), style);
        }
    }
    styles
}

// ---- Talking to the live document ----------------------------------------

/// Put (or replace) a `<style>` block with a known id inside the live document.
fn inject_block(view: &WebView, id: &str, css: &str, first: bool) {
    let script = format!(
        "(function (css) {{
           let el = document.getElementById({id});
           if (!el) {{
             el = document.createElement('style');
             el.id = {id};
             {place}
           }}
           el.textContent = css;
         }})({css})",
        id = crate::js::string(id),
        css = crate::js::string(css),
        // The base block goes first so the per-document block can override it by document order.
        place = if first {
            "document.head.insertBefore(el, document.head.firstChild);"
        } else {
            "document.head.appendChild(el);"
        },
    );
    view.evaluate_javascript(&script, None, None, gtk::gio::Cancellable::NONE, |_| {});
}

/// Write the base stylesheet into the live document.
pub fn inject_base(view: &WebView, base: &Base, setup: PageSetup) {
    inject_block(view, STYLE_ID, &base_css(base, setup), true);
}

/// Write the per-document block into the live document.
pub fn inject_custom(view: &WebView, css: &str) {
    inject_block(view, CUSTOM_STYLE_ID, css, false);
}

// ---- The element the sidebar is aimed at ---------------------------------
//
// The DOM half of the sidebar. The marker is a class rather than a Rust-side node handle because
// WebKit gives us no node handles: everything crosses the boundary as a string, so the document
// itself has to remember which element is selected between one call and the next.

/// The class marking the element the sidebar is editing.
pub const CURSOR_CLASS: &str = "wg-cursor";

/// What the sidebar needs to know about the element under the cursor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selected {
    /// Lowercase tag name, e.g. `li`.
    pub tag: String,
    /// The selector that addresses *this element alone*, if it already has one — a minted
    /// `.wg-iN`, or its own `#id`. Empty when it has no handle yet; one is minted on Apply.
    pub instance: String,
    /// Whether the document already styles this element specifically. Piers' rule: an element with
    /// classes or ids that pertain to its CSS opens on "this instance" rather than "all".
    pub specific: bool,
    pub has_parent: bool,
    pub has_child: bool,
    /// The highest `wg-iN` number in the document, so a minted class cannot collide with one whose
    /// rule has since been deleted.
    pub highest_instance: u32,
}

/// Records are `\u{1f}`-separated. Simple, unambiguous for the fields involved (none can contain a
/// unit separator), and it keeps a JSON dependency out of a crate the OS has to vendor offline.
const FS: char = '\u{1f}';

impl Selected {
    pub fn parse(record: &str) -> Option<Selected> {
        let f: Vec<&str> = record.split(FS).collect();
        if f.len() != 6 || f[0].is_empty() {
            return None;
        }
        Some(Selected {
            tag: f[0].to_string(),
            instance: f[1].to_string(),
            specific: f[2] == "1",
            has_parent: f[3] == "1",
            has_child: f[4] == "1",
            highest_instance: f[5].parse().unwrap_or(0),
        })
    }
}

/// The shared JavaScript half: helpers the sidebar calls into. Injected once per load.
///
/// It lives as one string rather than five because every one of them needs `cursor()`, and a helper
/// defined in one `evaluate_javascript` call is not in scope for the next.
pub fn cursor_script() -> String {
    format!(
        "window.wgCursor = {{
           el: function () {{ return document.querySelector('.{cursor}'); }},
           clear: function () {{
             document.querySelectorAll('.{cursor}').forEach(function (e) {{
               e.classList.remove('{cursor}');
             }});
           }},
           highest: function () {{
             let max = 0;
             document.querySelectorAll('[class]').forEach(function (e) {{
               e.classList.forEach(function (c) {{
                 const m = /^{prefix}(\\d+)$/.exec(c);
                 if (m) max = Math.max(max, parseInt(m[1], 10));
               }});
             }});
             return max;
           }},
           handle: function (el) {{
             const own = Array.from(el.classList).find(function (c) {{
               return /^{prefix}\\d+$/.test(c);
             }});
             if (own) return '.' + own;
             if (el.id) return '#' + el.id;
             return '';
           }},
           /* Piers' rule: an element the document already styles specifically opens on
              'this instance'. That is true of a handle we minted, and of any id or class of its
              own that some stylesheet actually targets. */
           specific: function (el) {{
             if (Array.from(el.classList).some(function (c) {{ return /^{prefix}\\d+$/.test(c); }})) {{
               return true;
             }}
             const wanted = [];
             if (el.id) wanted.push('#' + el.id);
             el.classList.forEach(function (c) {{ if (!c.startsWith('wg-')) wanted.push('.' + c); }});
             if (!wanted.length) return false;
             for (const sheet of document.styleSheets) {{
               let rules;
               try {{ rules = sheet.cssRules; }} catch (e) {{ continue; }}
               if (!rules) continue;
               for (const rule of rules) {{
                 if (!rule.selectorText) continue;
                 const parts = rule.selectorText.split(',').map(function (s) {{ return s.trim(); }});
                 if (wanted.some(function (w) {{ return parts.includes(w); }})) return true;
               }}
             }}
             return false;
           }},
           /* A fingerprint of the document's CONTENT, used to tell whether anything was typed
              since a style change. Handles and editing markers are stripped out first, so
              minting `wg-i3` onto an element does not read as an edit -- only real content does.
              djb2 plus the length: a hash rather than the markup itself, because a document with
              embedded pictures is megabytes and this crosses the boundary on every undo. */
           fingerprint: function () {{
             let s = document.body ? document.body.innerHTML : '';
             s = s.replace(/\\s*\\bwg-(cursor|selected|i\\d+)\\b/g, '');
             /* Removing the tokens leaves `class=\"\"` behind on an element that had no class
                attribute at all before a handle was minted onto it. Without this the fingerprint
                MOVES when a handle appears, an undo reads that as a text edit, and it takes back
                the wrong thing -- measured, in exactly Piers' a1..a5 sequence. */
             s = s.replace(/\\s*class=\"\\s*\"/g, '');
             let h = 5381;
             for (let i = 0; i < s.length; i++) {{ h = (((h * 33) ^ s.charCodeAt(i)) >>> 0); }}
             return h + ':' + s.length;
           }},
           describe: function () {{
             const el = window.wgCursor.el();
             if (!el) return '';
             return [
               el.tagName.toLowerCase(),
               window.wgCursor.handle(el),
               window.wgCursor.specific(el) ? '1' : '0',
               (el.parentElement && el.parentElement !== document.documentElement) ? '1' : '0',
               el.firstElementChild ? '1' : '0',
               String(window.wgCursor.highest()),
             ].join('{fs}');
           }},
         }};",
        cursor = CURSOR_CLASS,
        prefix = INSTANCE_CLASS_PREFIX,
        fs = "\\u001f",
    )
}

/// Put the cursor on whatever is at these viewport coordinates, and describe it.
pub fn select_at_script(x: f64, y: f64) -> String {
    format!(
        "(function (x, y) {{
           window.wgCursor.clear();
           let el = document.elementFromPoint(x, y);
           /* elementFromPoint misses when the click landed on a text node's whitespace; the
              selection anchor is where the caret actually went. */
           if (!el) {{
             const sel = window.getSelection();
             const node = sel && sel.anchorNode;
             el = node ? (node.nodeType === 1 ? node : node.parentElement) : null;
           }}
           if (!el || el === document.documentElement || el.tagName === 'BODY') return '';
           el.classList.add('{cursor}');
           return window.wgCursor.describe();
         }})({x}, {y})",
        cursor = CURSOR_CLASS,
    )
}

/// Move the cursor to the parent (`up`) or the first element child, and describe where it landed.
pub fn move_cursor_script(up: bool) -> String {
    format!(
        "(function () {{
           const el = window.wgCursor.el();
           if (!el) return '';
           const next = {step};
           if (!next || next === document.documentElement || next.tagName === 'HTML') return '';
           el.classList.remove('{cursor}');
           next.classList.add('{cursor}');
           return window.wgCursor.describe();
         }})()",
        step = if up { "el.parentElement" } else { "el.firstElementChild" },
        cursor = CURSOR_CLASS,
    )
}

/// Give the cursor element a handle of its own, minting `class` if it has none, and return the
/// selector that addresses it.
pub fn claim_instance_script(class: &str) -> String {
    format!(
        "(function (minted) {{
           const el = window.wgCursor.el();
           if (!el) return '';
           const existing = window.wgCursor.handle(el);
           if (existing) return existing;
           el.classList.add(minted);
           return '.' + minted;
         }})({class})",
        class = crate::js::string(class),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> CustomStyles {
        let mut styles = CustomStyles::new();
        styles.insert(
            "h1".into(),
            TagStyle { colour: "#003366".into(), font_size: "2.2em".into(), ..Default::default() },
        );
        styles.insert(
            "img".into(),
            TagStyle {
                border: "1px solid #cccccc".into(),
                radius: "8px".into(),
                shadow: HOUSE_SHADOW.into(),
                float: "left".into(),
                ..Default::default()
            },
        );
        styles.insert(
            "p".into(),
            TagStyle { text_align: "justify".into(), margin: "6px".into(), ..Default::default() },
        );
        styles
    }

    #[test]
    fn custom_styles_round_trip_through_css() {
        let before = sample();
        let after = parse_custom_css(&custom_css(&before));
        assert_eq!(before, after, "what the panel writes must be what it reads back");
    }

    #[test]
    fn tags_are_emitted_in_document_order_not_hash_order() {
        let css = custom_css(&sample());
        let h1 = css.find("h1 {").expect("h1 present");
        let p = css.find("p {").expect("p present");
        let img = css.find("img {").expect("img present");
        assert!(h1 < p && p < img, "STYLEABLE_TAGS order, not HashMap order:\n{css}");
    }

    #[test]
    fn float_and_text_align_survive_the_round_trip() {
        // These two are why the panel exists for pictures at all.
        let parsed = parse_custom_css("img { float: right; }\np { text-align: justify; }");
        assert_eq!(parsed["img"].float, "right");
        assert_eq!(parsed["p"].text_align, "justify");
        assert_eq!(parse_custom_css(&custom_css(&parsed)), parsed);
    }

    #[test]
    fn empty_styles_produce_no_rules() {
        let mut styles = CustomStyles::new();
        styles.insert("p".into(), TagStyle::default());
        let css = custom_css(&styles);
        assert!(!css.contains("p {"), "a tag with nothing set writes no rule");
        assert!(parse_custom_css(&css).is_empty());
    }

    #[test]
    fn border_and_border_radius_are_not_confused() {
        // Both start with "border"; a sloppy parser would fold one into the other.
        let parsed = parse_custom_css("p { border: 2px solid #111111; border-radius: 12px; }");
        assert_eq!(parsed["p"].border, "2px solid #111111");
        assert_eq!(parsed["p"].radius, "12px");
    }

    #[test]
    fn documents_the_browser_wrote_still_load() {
        // The browser's block predates float/text-align; the older shape must still parse rather
        // than failing the whole rule.
        let parsed = parse_custom_css("h2 { color: #445566; font-size: 1.4em; }");
        assert_eq!(parsed["h2"].colour, "#445566");
        assert!(parsed["h2"].float.is_empty() && parsed["h2"].text_align.is_empty());
    }

    // ---- scoping: all instances vs this instance --------------------------------------------

    #[test]
    fn an_instance_rule_overrides_the_tag_rule_without_replacing_it() {
        // Piers' worked example: every picture gets a red border; one of them is then made green.
        // The green rule must be a single line, and the red one must survive untouched.
        let mut styles = CustomStyles::new();
        styles.insert("img".into(), TagStyle { border: "1px solid #cc0000".into(), ..Default::default() });
        styles.insert(".wg-i1".into(), TagStyle { border: "1px solid #00aa00".into(), ..Default::default() });
        let css = custom_css(&styles);
        assert!(css.contains("img { border: 1px solid #cc0000; }"), "{css}");
        assert!(css.contains(".wg-i1 { border: 1px solid #00aa00; }"), "{css}");
        // The override names only what it overrides — everything else still comes from `img`.
        let instance_line = css.lines().find(|l| l.starts_with(".wg-i1")).unwrap();
        assert_eq!(instance_line.matches(':').count(), 1, "one property only: {instance_line}");
        assert_eq!(parse_custom_css(&css), styles);
    }

    #[test]
    fn dropping_the_instance_rule_leaves_the_tag_rule_applying() {
        // "on [Apply] the element specific CSS is deleted leaving the page wide CSS to apply"
        let mut styles = CustomStyles::new();
        styles.insert("img".into(), TagStyle { border: "1px solid #cc0000".into(), ..Default::default() });
        styles.insert(".wg-i1".into(), TagStyle { border: "1px solid #00aa00".into(), ..Default::default() });
        styles.remove(".wg-i1");
        let css = custom_css(&styles);
        assert!(css.contains("img { border: 1px solid #cc0000; }"), "{css}");
        assert!(!css.contains("wg-i1"), "the override is gone entirely:\n{css}");
    }

    #[test]
    fn ids_are_valid_instance_selectors_and_classes_are_too() {
        assert!(is_valid_selector("img"));
        assert!(is_valid_selector(".wg-i7"));
        assert!(is_valid_selector("#logo"));
        assert!(is_instance_selector("#logo") && is_instance_selector(".wg-i7"));
        assert!(!is_instance_selector("img"));
        // Anything that would make the block un-reasonable-about is refused.
        assert!(!is_valid_selector("div > p"));
        assert!(!is_valid_selector("a:hover"));
        assert!(!is_valid_selector(".2cols"));
        assert!(!is_valid_selector("."));
        assert!(!is_valid_selector("marquee"));
    }

    #[test]
    fn an_id_keyed_rule_round_trips() {
        let mut styles = CustomStyles::new();
        styles.insert("#logo".into(), TagStyle { float: "right".into(), ..Default::default() });
        assert_eq!(parse_custom_css(&custom_css(&styles)), styles);
    }

    #[test]
    fn tag_rules_come_before_instance_rules_and_mints_sort_numerically() {
        let mut styles = CustomStyles::new();
        for sel in [".wg-i10", ".wg-i9", "#logo", "img", "p"] {
            styles.insert(sel.into(), TagStyle { float: "left".into(), ..Default::default() });
        }
        let css = custom_css(&styles);
        let at = |s: &str| css.find(s).unwrap_or_else(|| panic!("{s} missing:\n{css}"));
        assert!(at("p {") < at("img {"), "STYLEABLE_TAGS order");
        assert!(at("img {") < at(".wg-i9"), "tags before instances");
        assert!(at(".wg-i9") < at(".wg-i10"), "9 before 10, not lexicographic");
        assert!(at(".wg-i10") < at("#logo"), "minted handles before other selectors");
    }

    #[test]
    fn a_minted_class_never_collides_with_one_already_in_the_document() {
        let mut styles = CustomStyles::new();
        styles.insert(".wg-i3".into(), TagStyle::default());
        // Nothing in the DOM: follow the CSS.
        assert_eq!(next_instance_class(&styles, 0), "wg-i4");
        // An element in the DOM carries a higher one whose rule was deleted — do not reuse it.
        assert_eq!(next_instance_class(&styles, 7), "wg-i8");
        assert_eq!(next_instance_class(&CustomStyles::new(), 0), "wg-i1");
    }

    #[test]
    fn a_v1_block_still_loads_and_is_rewritten_as_v2() {
        let v1 = "/* webgen-doc-custom v1 — written by the document style panel */\nh1 { color: #003366; }\n";
        let parsed = parse_custom_css(v1);
        assert_eq!(parsed["h1"].colour, "#003366");
        assert!(custom_css(&parsed).contains("v2"));
    }

    #[test]
    fn bold_and_underline_round_trip() {
        // Piers' use case: all paragraphs bold, then one of them bold AND underlined.
        let mut styles = CustomStyles::new();
        styles.insert("p".into(), TagStyle { font_weight: "bold".into(), ..Default::default() });
        styles.insert(
            ".wg-i1".into(),
            TagStyle {
                font_weight: "bold".into(),
                text_decoration: "underline".into(),
                ..Default::default()
            },
        );
        let css = custom_css(&styles);
        assert!(css.contains("p { font-weight: bold; }"), "{css}");
        assert!(css.contains(".wg-i1 { font-weight: bold; text-decoration: underline; }"), "{css}");
        assert_eq!(parse_custom_css(&css), styles);
    }

    #[test]
    fn an_explicit_normal_is_kept_because_it_undoes_an_inherited_bold() {
        let parsed = parse_custom_css("li { font-weight: normal; font-style: italic; }");
        assert_eq!(parsed["li"].font_weight, "normal");
        assert_eq!(parsed["li"].font_style, "italic");
        assert!(!parsed["li"].is_empty());
    }

    #[test]
    fn a_zero_width_border_writes_no_declaration_at_all() {
        // Piers' rule: "0px = remove all border code from that stanza of CSS".
        assert_eq!(compose_border(0, "solid", "#cc0000"), "");
        let style = TagStyle { border: compose_border(0, "solid", "#cc0000"), ..Default::default() };
        assert!(style.is_empty(), "nothing else set, so the rule disappears too");
        let mut styles = CustomStyles::new();
        styles.insert("img".into(), TagStyle {
            colour: "#111111".into(),
            border: compose_border(0, "dashed", "#cc0000"),
            ..Default::default()
        });
        let css = custom_css(&styles);
        assert!(css.contains("color: #111111;"), "{css}");
        assert!(!css.contains("border"), "no border code survives a 0px width:\n{css}");
    }

    #[test]
    fn the_three_border_pickers_round_trip() {
        for (w, st, c) in [(1, "solid", "#cc0000"), (3, "dashed", "#00aa00"), (12, "double", "#0000ff")] {
            let composed = compose_border(w, st, c);
            assert_eq!(composed, format!("{w}px {st} {c}"));
            assert_eq!(parse_border(&composed), (w, st.to_string(), c.to_string()));
        }
    }

    #[test]
    fn a_hand_written_border_is_read_into_the_pickers() {
        // Order does not matter, names are resolved, and case is irrelevant.
        assert_eq!(parse_border("1px solid red"), (1, "solid".into(), "#ff0000".into()));
        assert_eq!(parse_border("DOTTED 2px #ABCDEF"), (2, "dotted".into(), "#ABCDEF".into()));
        assert_eq!(parse_border("none"), (0, "solid".into(), "#000000".into()));
        assert_eq!(parse_border(""), (0, "solid".into(), "#000000".into()));
        // An unknown keyword is not mistaken for a style.
        assert_eq!(parse_border("4px groove rebeccapurple").0, 4);
        assert_eq!(parse_border("4px groove rebeccapurple").1, "groove");
    }

    #[test]
    fn a_selected_element_record_round_trips() {
        let record = "li\u{1f}.wg-i2\u{1f}1\u{1f}1\u{1f}0\u{1f}5";
        let s = Selected::parse(record).expect("parses");
        assert_eq!(s.tag, "li");
        assert_eq!(s.instance, ".wg-i2");
        assert!(s.specific && s.has_parent && !s.has_child);
        assert_eq!(s.highest_instance, 5);
        // An empty answer means "nothing is selected", not a default-shaped element.
        assert!(Selected::parse("").is_none());
        assert!(Selected::parse("li\u{1f}\u{1f}0").is_none());
    }

    #[test]
    fn an_element_with_no_handle_yet_parses_as_having_none() {
        let s = Selected::parse("p\u{1f}\u{1f}0\u{1f}1\u{1f}1\u{1f}0").expect("parses");
        assert!(s.instance.is_empty());
        assert!(!s.specific);
    }

    #[test]
    fn foreign_css_is_ignored_rather_than_half_understood() {
        let css = "body { color: red; }\nh1 { color: #112233; }\n@media print { p { color: blue } }";
        let parsed = parse_custom_css(css);
        assert_eq!(parsed.len(), 1, "only styleable tags are taken");
        assert_eq!(parsed["h1"].colour, "#112233");
        assert!(!parsed.contains_key("body"));
    }

    #[test]
    fn the_base_stylesheet_carries_the_page_geometry_it_was_given() {
        let base = Base {
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
        };
        let setup = PageSetup { paper: crate::page::Paper::A5, top: 10.0, right: 12.0, bottom: 10.0, left: 12.0 };
        let css = base_css(&base, setup);
        assert!(css.contains("size: A5;"), "{css}");
        assert!(css.contains("margin: 10mm 12mm 10mm 12mm;"), "{css}");
        assert!(css.contains("max-width: 124mm"), "148 - 12 - 12:\n{css}");
        // Both page-break classes are styled, so 0.2-era documents still break.
        assert!(css.contains(PAGE_BREAK_CLASS) && css.contains(LEGACY_PAGE_BREAK_CLASS));
    }
}
