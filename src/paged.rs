//! Pages view: the read-only two-page spread.
//!
//! Piers, 2026-08-06: *"default a) you only see the page you are working on. toggle two and you
//! see them side by side. all documents have [<] [>] buttons to go to the next page(s). So in
//! side by side view we see 1,2 > 2,3 > 3,4 < 2,3 etc."* — with the whole-document reading case
//! already served by Read-as-PDF.
//!
//! The page you are WORKING on is the ordinary editing view — continuous flow, markers visible.
//! This mode is for LOOKING: it splits the document at its page markers (the gate's own output,
//! so the spread and the PDF agree), wraps each page's blocks in a sheet, lays two sheets side by
//! side, and walks them with [<] [>]. It is **read-only by design**: editing across a sheet
//! boundary is the one dragon this feature is shaped to avoid, so entering the view drops
//! editability and leaving restores it. The wrapping is pure DOM re-parenting — entering and
//! leaving moves the same nodes, nothing is cloned, and save is refused while wrapped so the
//! wrapper divs can never leak into a file.

/// Wrap the document into sheets and show the first spread. Returns the page count.
pub fn enter_script(page_px: f64) -> String {
    format!(
        r##"(function() {{
    var body = document.body;
    if (!body || document.getElementById("wg-pages-root")) return "0";
    function isBreak(el) {{ return el.classList && (el.classList.contains("webgen-page-break") || el.classList.contains("pagebreak")); }}
    var kids = Array.prototype.slice.call(body.childNodes);
    var root = document.createElement("div");
    root.id = "wg-pages-root";
    var sheet = document.createElement("div");
    sheet.className = "wg-sheet";
    root.appendChild(sheet);
    for (var i = 0; i < kids.length; i++) {{
        var el = kids[i];
        sheet.appendChild(el);
        if (el.nodeType === 1 && isBreak(el)) {{
            sheet = document.createElement("div");
            sheet.className = "wg-sheet";
            root.appendChild(sheet);
        }}
    }}
    var css = document.createElement("style");
    css.id = "wg-paged-css";
    css.textContent =
        "body.wg-paged {{ max-width: none !important; }}" +
        "#wg-pages-root {{ display: flex; gap: 12mm; justify-content: center; align-items: flex-start; }}" +
        "#wg-pages-root .wg-sheet {{ display: none; flex: 0 0 auto; width: calc(100% / 2 - 8mm); " +
        "min-height: {page_px:.0}px; background: inherit; box-shadow: 0 0 0 1px rgba(0,0,0,0.25); " +
        "padding: 6mm; box-sizing: border-box; overflow: hidden; }}" +
        "#wg-pages-root .wg-sheet.wg-show {{ display: block; }}";
    document.head.appendChild(css);
    body.appendChild(root);
    body.classList.add("wg-paged");
    return String(root.children.length);
}})()"##
    )
}

/// Show the spread starting at `first` (0-based): pages `first` and `first+1`.
pub fn show_script(first: usize) -> String {
    format!(
        r##"(function() {{
    var root = document.getElementById("wg-pages-root");
    if (!root) return "0";
    var sheets = root.children;
    for (var i = 0; i < sheets.length; i++)
        sheets[i].classList.toggle("wg-show", i === {first} || i === {first} + 1);
    window.scrollTo(0, 0);
    return String(sheets.length);
}})()"##
    )
}

/// Unwrap: every node goes back to the body in order; the machinery vanishes without trace.
pub fn exit_script() -> String {
    r#"(function() {
    var root = document.getElementById("wg-pages-root");
    if (!root) return "";
    var body = document.body;
    var sheets = Array.prototype.slice.call(root.children);
    for (var i = 0; i < sheets.length; i++) {
        var nodes = Array.prototype.slice.call(sheets[i].childNodes);
        for (var j = 0; j < nodes.length; j++) body.insertBefore(nodes[j], root);
    }
    body.removeChild(root);
    var css = document.getElementById("wg-paged-css");
    if (css) css.parentNode.removeChild(css);
    body.classList.remove("wg-paged");
    return "";
})()"#
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scripts_are_symmetric_about_the_same_ids() {
        let enter = enter_script(900.0);
        let exit = exit_script();
        for id in ["wg-pages-root", "wg-paged-css", "wg-paged"] {
            assert!(enter.contains(id), "enter lacks {id}");
            assert!(exit.contains(id), "exit lacks {id}");
        }
        assert!(show_script(2).contains("=== 2 || i === 2 + 1"));
    }
}
