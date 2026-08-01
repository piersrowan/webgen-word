//! Page geometry — paper size and margins.
//!
//! **This module exists because CSS cannot do it.** The obvious design is to let the document carry
//! `@page { size: A4; margin: 20mm; }` and be done. That does not work: WebKit's GTK print path
//! ignores `@page` entirely. Measured — asking for A4 with 45mm/40mm margins produced US Letter with
//! 10mm/7mm margins, which are GTK's defaults, not anything the stylesheet said.
//!
//! So page setup has to be application state, applied to a `gtk::PageSetup` at print time, and it
//! has to be in the UI because the user cannot reach it any other way.
//!
//! ## Where a page setup lives
//!
//! In **two** places, deliberately:
//!
//! - the **document** carries its own, in `<meta name="webgen-page" content="A4;20;20;20;20">`.
//!   Until 0.3.0 it did not, which meant opening a CV authored at A5 and printing it produced A4:
//!   the `@page` rule was written into the file and never read back, so the file described a
//!   geometry the app then ignored.
//! - the **registry** carries the default a *new* document starts from (CONTRACT.md §2), so a house
//!   style set once in System Settings is what every new document gets.
//!
//! The meta wins when present. That is the same base-plus-per-document split the stylesheet uses.

use crate::settings::Settings;

/// The meta element that carries a document's own page setup.
pub const PAGE_META: &str = "webgen-page";

/// Paper sizes worth offering. Deliberately short: the point is A4-or-Letter, not a catalogue.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Paper {
    A4,
    Letter,
    Legal,
    A5,
}

impl Paper {
    pub const ALL: [Paper; 4] = [Paper::A4, Paper::Letter, Paper::Legal, Paper::A5];

    pub fn label(self) -> &'static str {
        match self {
            Paper::A4 => "A4",
            Paper::Letter => "US Letter",
            Paper::Legal => "US Legal",
            Paper::A5 => "A5",
        }
    }

    /// The short name used in the document's meta and in the registry. Stable — documents on disk
    /// depend on it, so it is not the display label.
    pub fn key(self) -> &'static str {
        match self {
            Paper::A4 => "A4",
            Paper::Letter => "Letter",
            Paper::Legal => "Legal",
            Paper::A5 => "A5",
        }
    }

    pub fn from_key(s: &str) -> Option<Paper> {
        Self::ALL.iter().copied().find(|p| p.key().eq_ignore_ascii_case(s.trim()))
    }

    /// The CSS `@page size` keyword.
    pub fn css_size(self) -> &'static str {
        match self {
            Paper::A4 => "A4",
            Paper::Letter => "Letter",
            Paper::Legal => "Legal",
            Paper::A5 => "A5",
        }
    }

    /// The GTK/PWG name. These are the standard identifiers `gtk::PaperSize` understands.
    fn gtk_name(self) -> &'static str {
        match self {
            Paper::A4 => "iso_a4",
            Paper::Letter => "na_letter",
            Paper::Legal => "na_legal",
            Paper::A5 => "iso_a5",
        }
    }

    /// Full paper height in millimetres, for drawing a page-shaped area on screen.
    pub fn height_mm(self) -> f64 {
        match self {
            Paper::A4 => 297.0,
            Paper::Letter => 279.4,
            Paper::Legal => 355.6,
            Paper::A5 => 210.0,
        }
    }

    /// Full paper width in millimetres.
    pub fn width_mm(self) -> f64 {
        match self {
            Paper::A4 => 210.0,
            Paper::Letter => 215.9,
            Paper::Legal => 215.9,
            Paper::A5 => 148.0,
        }
    }

    pub fn from_index(i: u32) -> Paper {
        *Self::ALL.get(i as usize).unwrap_or(&Paper::A4)
    }

    pub fn index(self) -> u32 {
        Self::ALL.iter().position(|p| *p == self).unwrap_or(0) as u32
    }
}

/// Everything that decides what the printed page looks like.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PageSetup {
    pub paper: Paper,
    /// Millimetres. One value per edge because CVs routinely want a wider left margin for binding.
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

impl Default for PageSetup {
    fn default() -> Self {
        // A4 with 20mm all round: the sane default for a document written in the anglophone world
        // outside the US, and the one a CV wants. Not Letter -- GTK's default is Letter and that is
        // exactly the trap this module exists to close.
        PageSetup { paper: Paper::A4, top: 20.0, right: 20.0, bottom: 20.0, left: 20.0 }
    }
}

impl PageSetup {
    /// The default a *new* document starts from, per the registry.
    pub fn from_settings(settings: &Settings) -> Self {
        let d = PageSetup::default();
        PageSetup {
            paper: Paper::from_key(&settings.string("page_paper", d.paper.key())).unwrap_or(d.paper),
            top: settings.i64("page_margin_top_mm", d.top as i64) as f64,
            right: settings.i64("page_margin_right_mm", d.right as i64) as f64,
            bottom: settings.i64("page_margin_bottom_mm", d.bottom as i64) as f64,
            left: settings.i64("page_margin_left_mm", d.left as i64) as f64,
        }
        .clamped()
    }

    /// Make this the default for new documents.
    pub fn save_as_default(self, settings: &Settings) {
        settings.set_string("page_paper", self.paper.key());
        settings.set_i64("page_margin_top_mm", self.top as i64);
        settings.set_i64("page_margin_right_mm", self.right as i64);
        settings.set_i64("page_margin_bottom_mm", self.bottom as i64);
        settings.set_i64("page_margin_left_mm", self.left as i64);
    }

    /// The `content` of the document's `<meta name="webgen-page">`.
    pub fn to_meta(self) -> String {
        format!(
            "{};{};{};{};{}",
            self.paper.key(),
            self.top as i64,
            self.right as i64,
            self.bottom as i64,
            self.left as i64
        )
    }

    /// Read one back. Anything malformed yields `None` rather than a half-applied geometry.
    pub fn from_meta(content: &str) -> Option<PageSetup> {
        let parts: Vec<&str> = content.split(';').map(str::trim).collect();
        if parts.len() != 5 {
            return None;
        }
        let paper = Paper::from_key(parts[0])?;
        let mut mm = [0.0f64; 4];
        for (slot, text) in mm.iter_mut().zip(&parts[1..]) {
            *slot = text.parse::<f64>().ok()?;
        }
        Some(PageSetup { paper, top: mm[0], right: mm[1], bottom: mm[2], left: mm[3] }.clamped())
    }

    /// Keep margins inside what the dialog offers and what the paper can hold.
    fn clamped(mut self) -> Self {
        let limit = (self.paper.width_mm() / 2.0 - 20.0).max(0.0);
        for m in [&mut self.top, &mut self.right, &mut self.bottom, &mut self.left] {
            *m = m.clamp(0.0, 60.0);
        }
        // Side margins may not eat the page.
        if self.left + self.right > limit * 2.0 {
            let scale = (limit * 2.0) / (self.left + self.right);
            self.left *= scale;
            self.right *= scale;
        }
        self
    }

    /// Build the `gtk::PageSetup` the print operation actually obeys.
    pub fn to_gtk(self) -> gtk::PageSetup {
        let ps = gtk::PageSetup::new();
        ps.set_paper_size(&gtk::PaperSize::new(Some(self.paper.gtk_name())));
        ps.set_top_margin(self.top, gtk::Unit::Mm);
        ps.set_right_margin(self.right, gtk::Unit::Mm);
        ps.set_bottom_margin(self.bottom, gtk::Unit::Mm);
        ps.set_left_margin(self.left, gtk::Unit::Mm);
        ps
    }

    /// The on-screen page width in CSS millimetres, so the editor shows a page-shaped column rather
    /// than a full-window text slab. Purely cosmetic — printing does not consult it.
    pub fn content_width_mm(self) -> f64 {
        (self.paper.width_mm() - self.left - self.right).max(40.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_setup_round_trips_through_the_document_meta() {
        let setup = PageSetup { paper: Paper::A5, top: 12.0, right: 14.0, bottom: 16.0, left: 18.0 };
        assert_eq!(PageSetup::from_meta(&setup.to_meta()), Some(setup));
    }

    #[test]
    fn the_default_survives_the_trip_too() {
        let d = PageSetup::default();
        assert_eq!(PageSetup::from_meta(&d.to_meta()), Some(d));
        assert_eq!(d.to_meta(), "A4;20;20;20;20");
    }

    #[test]
    fn malformed_meta_is_refused_rather_than_half_applied() {
        assert!(PageSetup::from_meta("").is_none());
        assert!(PageSetup::from_meta("A4;20;20;20").is_none());
        assert!(PageSetup::from_meta("Foolscap;20;20;20;20").is_none());
        assert!(PageSetup::from_meta("A4;20;20;20;wide").is_none());
    }

    #[test]
    fn paper_keys_are_stable_and_case_insensitive() {
        // Documents on disk hold these strings, so they are not the display labels.
        assert_eq!(Paper::from_key("letter"), Some(Paper::Letter));
        assert_eq!(Paper::Letter.key(), "Letter");
        assert_eq!(Paper::Letter.label(), "US Letter");
    }

    #[test]
    fn margins_that_would_eat_the_page_are_pulled_back() {
        let silly = PageSetup { paper: Paper::A5, top: 0.0, right: 60.0, bottom: 0.0, left: 60.0 };
        let got = PageSetup::from_meta(&silly.to_meta()).unwrap();
        assert!(got.content_width_mm() >= 40.0, "{got:?}");
    }

    #[test]
    fn content_width_follows_the_paper() {
        let a4 = PageSetup::default();
        assert_eq!(a4.content_width_mm(), 170.0);
    }
}
