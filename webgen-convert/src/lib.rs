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

pub use docx::docx_to_html;

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
