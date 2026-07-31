//! Page geometry — paper size and margins.
//!
//! **This module exists because CSS cannot do it.** The obvious design is to let the document carry
//! `@page { size: A4; margin: 20mm; }` and be done. That does not work: WebKit's GTK print path
//! ignores `@page` entirely. Measured — asking for A4 with 45mm/40mm margins produced US Letter with
//! 10mm/7mm margins, which are GTK's defaults, not anything the stylesheet said.
//!
//! So page setup has to be applications state, applied to a `gtk::PageSetup` at print time, and it
//! has to be in the UI because the user cannot reach it any other way.


/// Paper sizes worth offering. Deliberately short: the point is A4-or-Letter, not a catalogue.
#[derive(Clone, Copy, PartialEq, Eq)]
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

    /// The GTK/PWG name. These are the standard identifiers `gtk::PaperSize` understands.
    fn gtk_name(self) -> &'static str {
        match self {
            Paper::A4 => "iso_a4",
            Paper::Letter => "na_letter",
            Paper::Legal => "na_legal",
            Paper::A5 => "iso_a5",
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
#[derive(Clone, Copy)]
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

    /// The on-screen page width in CSS millimetres, so the editor can show a page-shaped column
    /// rather than a full-window text slab. Purely cosmetic -- printing does not consult it.
    pub fn content_width_mm(self) -> f64 {
        let full = match self.paper {
            Paper::A4 => 210.0,
            Paper::Letter => 215.9,
            Paper::Legal => 215.9,
            Paper::A5 => 148.0,
        };
        (full - self.left - self.right).max(40.0)
    }
}
