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
/* The picture the Picture menu is aimed at. Editing only — stripped before saving. */
.wg-selected {{ outline: 3px solid {link}; outline-offset: 3px; }}
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
    "table", "td", "th", "img",
];

/// The properties offered per tag. Kept deliberately short: this is document preparation, not a CSS
/// editor — every one of these is a thing a writer wants, and nothing here can break layout.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TagStyle {
    pub font_family: String,
    pub font_size: String,  // e.g. "1.2em" or "18px"
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

/// One document's overrides: tag → style. Ordered by [`STYLEABLE_TAGS`] on the way out.
pub type CustomStyles = HashMap<String, TagStyle>;

/// Render the overrides in the fixed layout — one tag per line, properties in a fixed order.
pub fn custom_css(styles: &CustomStyles) -> String {
    let mut out = String::from("/* webgen-doc-custom v1 — written by the document style panel */\n");
    for tag in STYLEABLE_TAGS {
        let Some(style) = styles.get(*tag) else { continue };
        let declarations = style.declarations();
        if declarations.is_empty() {
            continue;
        }
        let body = declarations
            .iter()
            .map(|(p, v)| format!("{p}: {v};"))
            .collect::<Vec<_>>()
            .join(" ");
        out.push_str(&format!("{tag} {{ {body} }}\n"));
    }
    out
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
        if !STYLEABLE_TAGS.contains(&tag) {
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

/// Read the per-document block back out, so the panel opens showing what the document actually has.
pub fn read_custom<F: FnOnce(CustomStyles) + 'static>(view: &WebView, done: F) {
    let script = format!(
        "(function () {{
           const el = document.getElementById({});
           return el ? el.textContent : '';
         }})()",
        crate::js::string(CUSTOM_STYLE_ID)
    );
    view.evaluate_javascript(&script, None, None, gtk::gio::Cancellable::NONE, move |result| {
        let css = result.map(|v| v.to_str().to_string()).unwrap_or_default();
        done(parse_custom_css(&css));
    });
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
