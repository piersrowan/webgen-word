//! $format -> HTML. docx first.
//!
//! The shape of the deal: a converter takes the FOREIGN bytes and returns HTML plus a list of
//! extracted assets (images), and nothing else. No IO, no GTK, no opinions about where files
//! live — the caller owns paths and the caller's sanitiser owns trust. webgen-word feeds the
//! result through the same `sanitise::clean` it runs on any opened file, so a hostile docx gets
//! exactly the scepticism a hostile html does.
//!
//! Fidelity is deliberately semantic, not visual: paragraphs, headings, bold/italic/underline,
//! lists (including numbering carried by a paragraph STYLE, which is how Word templates do
//! bullets), tables with merged cells, hyperlinks, pictures. Page-layout theatre — text boxes,
//! columns, floating frames — is out of scope; that is what LibreOffice remains installed for.

mod docx;

pub use docx::{docx_to_html, docx_to_segments, render_table_html};

/// One block-level piece of a converted document, in order. Consumers that only want markup can
/// use [`docx_to_html`]; a consumer with its own table machinery (webgen-word's JSON-model table
/// blocks) takes segments and decides per table whether to adopt it natively.
pub enum Segment {
    Html(String),
    Table(DocTable),
}

/// A table as the source document meant it: spans resolved, merge-continuation cells removed,
/// visual styling carried as DATA (never inline CSS — the consumer turns it into sheet rules).
pub struct DocTable {
    /// Unique per conversion (nested tables included) — the `wg-conv-tN` scope id, so a consumer
    /// rendering some tables itself can never collide with ones the converter rendered inline.
    pub seq: u32,
    pub rows: Vec<Vec<DocCell>>,
    /// The source declared visible borders (docx `tblBorders` with any non-nil edge).
    pub bordered: bool,
    /// Column widths in millimetres from `w:tblGrid`, one per grid column. Empty when the
    /// document states none. These are what stop a converted form re-wrapping its cells and
    /// drifting down the page (Piers, 2026-08-06).
    pub col_widths_mm: Vec<f64>,
}

/// What [`docx_to_segments`] produces.
pub struct ConvertedSegments {
    pub segments: Vec<Segment>,
    pub assets: Vec<(String, Vec<u8>)>,
    pub notes: Vec<String>,
    /// The document's OWN page geometry from `w:sectPr`, in millimetres. Ignoring it was why a
    /// converted form printed to more pages than Word or LibreOffice give it: the app's A4/20mm
    /// default is narrower than the ~15/12/9mm these templates actually use, so ~25% less content
    /// fitted per page (2026-08-06).
    pub page: Option<DocPage>,
}

/// Page size and margins, millimetres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DocPage {
    pub width_mm: f64,
    pub height_mm: f64,
    pub top_mm: f64,
    pub right_mm: f64,
    pub bottom_mm: f64,
    pub left_mm: f64,
}

pub struct DocCell {
    /// The cell's content as sanitisable HTML (paragraphs, lists, nested tables…).
    pub html: String,
    /// `Some((text, bold))` when the content is exactly one plain paragraph — at most wholly
    /// bold — so a model-based consumer can adopt the cell losslessly. `None` means "complex";
    /// keep the html.
    pub simple: Option<(String, bool)>,
    pub colspan: u32,
    pub rowspan: u32,
    /// Background fill as `#rrggbb`, from docx `w:shd w:fill` (never "auto").
    pub fill: Option<String>,
}

/// What a conversion produces.
#[derive(Debug)]
pub struct Converted {
    /// Body-level HTML (no <html>/<head> wrapper — the caller decides the document shell).
    pub body_html: String,
    /// Extracted binary assets, `(file name, bytes)`. Names are already unique and safe; the
    /// caller writes them into its pictures folder. `body_html` references them as
    /// `{asset_dir}/{name}` using the `asset_dir` passed to the converter.
    pub assets: Vec<(String, Vec<u8>)>,
    /// Human-readable notes about content that did not survive (dropped drawings, etc.).
    pub notes: Vec<String>,
}
