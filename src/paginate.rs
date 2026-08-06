//! The page-marker gate.
//!
//! Piers, 2026-08-06: *"A new page is ONLY a new page when there is a deliberate marker in the
//! document… we let the user add them, sure, we respect that, but we hit 94% of our page end
//! window and we add it automatically. We have some marker where they can beg for extra lines but
//! that is it. We deal in page markers, they deal in 'OK then' or 'I just need to finish this
//! sentence'."*
//!
//! So pagination here is not live reflow guessing — it is markers IN the document:
//!
//! - The user's own breaks are the existing `<hr class="webgen-page-break">` (Ctrl+Return).
//! - The editor adds `<hr class="webgen-page-break wg-auto">` when the content since the last
//!   marker crosses **94%** of the page window (page height minus margins, from the document's own
//!   PageSetup). It breaks BEFORE the block that crosses — a block is never split — and a block
//!   taller than a page simply overflows rather than trapping the checker in marker spam.
//! - A slack marker (`<hr class="wg-page-slack">`, "let this page run long") raises that page's
//!   gate to 110% — "I just need to finish this sentence."
//! - Auto markers are EDITOR-OWNED: Repaginate deletes every `.wg-auto` and lets the checker lay
//!   them out fresh. User markers are never touched. Because the markers are ordinary elements,
//!   they save with the file and the print path's `break-before: page` obeys them by construction
//!   — the editor and the PDF cannot disagree.
//!
//! The checker runs on a timer from `main.rs`, only while the window is active, and reports
//! `"inserted,pages"` so the status line can mention the count when it changes.

use crate::page::PageSetup;

/// Class marking an auto-inserted page break (alongside the ordinary page-break class).
pub const AUTO_CLASS: &str = "wg-auto";
/// The slack marker's class.
pub const SLACK_CLASS: &str = "wg-page-slack";

/// The page window in CSS pixels: printable height at the browser's 96dpi.
pub fn inner_height_px(setup: &PageSetup) -> f64 {
    ((setup.paper.height_mm() - setup.top - setup.bottom).max(40.0)) * 96.0 / 25.4
}

/// One pass of the gate: insert auto markers where content crosses the window; count pages.
/// Returns a script whose value is `"inserted,pages"`.
pub fn check_script(page_px: f64) -> String {
    format!(
        r#"(function() {{
    var PAGE = {page_px:.1};
    var body = document.body;
    if (!body) return "0,1";
    function isBreak(el) {{ return el.classList && (el.classList.contains("webgen-page-break") || el.classList.contains("pagebreak")); }}
    function isSlack(el) {{ return el.classList && el.classList.contains("{slack}"); }}
    var kids = Array.prototype.slice.call(body.children);
    var pageTop = 0, slack = false, inserted = 0, pages = 1, placed = 0;
    for (var i = 0; i < kids.length; i++) {{
        var el = kids[i];
        if (isBreak(el)) {{ pageTop = el.offsetTop + el.offsetHeight; slack = false; placed = 0; pages++; continue; }}
        if (isSlack(el)) {{ slack = true; continue; }}
        if (el.offsetHeight === 0) continue; /* hidden or empty: never a reason to break */
        var bottom = el.offsetTop + el.offsetHeight;
        var limit = PAGE * (slack ? 1.10 : 0.94);
        /* Never break with nothing on the page yet: a document whose FIRST block is taller than
           the window (an assessment form opening on a big table) would otherwise get a break at
           position zero and print a blank first page (2026-08-06). A block with no room above it
           starts where it starts and overflows. */
        if (placed === 0) {{
            placed++;
            if (el.offsetHeight > limit) {{ pageTop = bottom; slack = false; placed = 0;
                pages += Math.max(0, Math.ceil(el.offsetHeight / PAGE) - 1); }}
            continue;
        }}
        placed++;
        if (bottom - pageTop > limit && el.offsetTop > pageTop) {{
            /* Idempotency is the whole game: a break already sitting immediately above this
               block means the gate has ALREADY answered here — inserting again every pass is
               the 173-page runaway (2026-08-06). */
            var prev = el.previousElementSibling;
            if (!(prev && isBreak(prev))) {{
                var hr = document.createElement("hr");
                hr.className = "webgen-page-break {auto}";
                body.insertBefore(hr, el);
                pageTop = hr.offsetTop + hr.offsetHeight;
                slack = false;
                inserted++;
                pages++;
            }}
        }}
        /* A block taller than the page owns its page(s) and overflows honestly: the next
           measurement starts BELOW it, or everything after it would break one line each. The
           count must own them too — a table spanning three sheets IS three pages, and growing
           it should move the number even though no marker can land inside a block (2026-08-06:
           "new lines inside a table didn't count"). */
        if (el.offsetHeight > limit) {{
            pageTop = bottom;
            slack = false;
            placed = 0;
            pages += Math.max(0, Math.ceil(el.offsetHeight / PAGE) - 1);
        }}
    }}
    return inserted + "," + pages;
}})()"#,
        slack = SLACK_CLASS,
        auto = AUTO_CLASS,
    )
}

/// Repaginate: every auto marker dies, then the gate lays them out fresh in the same pass.
/// User markers are untouched — they are the user's. Returns `"inserted,pages"`.
pub fn repaginate_script(page_px: f64) -> String {
    format!(
        r#"(function() {{
    var autos = document.querySelectorAll("hr.{auto}");
    for (var i = 0; i < autos.length; i++) autos[i].parentNode.removeChild(autos[i]);
    return {check};
}})()"#,
        auto = AUTO_CLASS,
        check = check_script(page_px),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::{PageSetup, Paper};

    #[test]
    fn the_window_is_printable_height_at_css_dpi() {
        // A4 297mm minus 20+20 margins = 257mm -> 257/25.4*96 ≈ 971px.
        let s = PageSetup { paper: Paper::A4, top: 20.0, right: 20.0, bottom: 20.0, left: 20.0 };
        let px = inner_height_px(&s);
        assert!((px - 971.3).abs() < 1.0, "{px}");
    }

    #[test]
    fn the_scripts_carry_the_gate_numbers() {
        let js = check_script(1000.0);
        assert!(js.contains("0.94"), "the 94% window: {js}");
        assert!(js.contains("1.10"), "the slack allowance: {js}");
        assert!(js.contains("webgen-page-break wg-auto"), "{js}");
        let re = repaginate_script(1000.0);
        assert!(re.contains("hr.wg-auto"), "{re}");
    }
}
