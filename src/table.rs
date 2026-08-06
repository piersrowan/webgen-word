//! Tables — the model, the JSON it is stored as, and the block it becomes in the document.
//!
//! Piers, 2026-08-01: *"Tables are painful to get right but essential."* They are, and the reason is
//! that a table is the one thing in a document that is genuinely two-dimensional. A merged cell
//! means the row below it has fewer cells than columns, so "insert a column" is not "push a cell
//! into every row" and "what is at row 3, column 2" is not `rows[3][2]`. Everything here exists to
//! keep that straight, and the tests are where it is actually kept straight.
//!
//! ## What a table is on disk
//!
//! ```text
//! <!-- table block -->
//! <style>
//!   .wg-t1 { … }              /* scoped to THIS table */
//! </style>
//! <table class="wg-t1" data-wg-table="{…json…}">
//!   <thead><tr><th>Heading</th></tr></thead>
//!   <tbody><tr><td>Cell 1 Row 2</td></tr></tbody>
//! </table>
//! <!-- END table block -->
//! ```
//!
//! Three things about that shape are load-bearing:
//!
//! - **The comments delimit what gets replaced.** Editing a table regenerates everything between
//!   them; nothing tries to patch the markup in place.
//! - **The CSS is scoped to `.wg-tN`.** The style block sits in the document, so an unscoped
//!   `table { … }` would restyle every table in the file rather than this one. The editor talks in
//!   terms of `table`, `thead tr`, `td` and so on; the prefix is added on the way out.
//! - **The JSON is the truth, and it lives in `data-wg-table`.** The rendered HTML is a projection
//!   of it. Re-opening a table parses the attribute rather than the markup, so the editor never has
//!   to guess what a `rowspan` meant — the same reason the style block is written in a fixed layout
//!   rather than parsed as arbitrary CSS. It is not in the comment because a cell containing `-->`
//!   would end the comment early, and not in a `<script type="application/json">` because a word
//!   processor that strips script should not make exceptions to that.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::docstyle::TagStyle;

/// Opening marker. Everything up to [`END_MARKER`] is regenerated when a table is saved.
pub const START_MARKER: &str = "<!-- table block -->";
pub const END_MARKER: &str = "<!-- END table block -->";

/// The attribute carrying the model.
pub const DATA_ATTR: &str = "data-wg-table";

/// The class prefix that scopes a table's CSS to that table.
pub const TABLE_CLASS_PREFIX: &str = "wg-t";

/// The selectors the table style editor offers, and how it names them.
///
/// Written as they are *thought about* — `thead tr`, `td` — and scoped to the table on the way out.
/// `:nth-child(odd|even)` is how banded rows are done, which is the one thing every table wants and
/// nothing else in the app offers.
pub const SELECTORS: &[(&str, &str)] = &[
    ("table", "The table"),
    ("caption", "Caption"),
    ("thead", "Head"),
    ("thead tr", "Head row"),
    ("th", "Heading cell"),
    ("tbody", "Body"),
    ("tbody tr", "Body row"),
    ("tbody tr:nth-child(odd)", "Body row — odd"),
    ("tbody tr:nth-child(even)", "Body row — even"),
    ("td", "Data cell"),
];

/// One cell. `colspan`/`rowspan` of 1 are the normal case and are not written out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cell {
    #[serde(default)]
    pub text: String,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub colspan: usize,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub rowspan: usize,
    #[serde(default, skip_serializing_if = "is_false")]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub underline: bool,
    /// `left` | `center` | `right` — empty means "whatever the CSS says".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub align: String,
    /// Background fill as a CSS colour (`#d9d9d9`), empty for none.
    ///
    /// Rendered as a `.cell_rN_cM` class on the cell plus a matching rule in the scoped style
    /// block — Piers's reserved per-cell form (2026-08-06), and NEVER an inline style: every
    /// visual fact stays in a sheet where it can be read and overridden. Rows and columns are
    /// numbered 1-based over the whole table's visual grid (spans resolved), head included.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fill: String,
}

fn one() -> usize {
    1
}
fn is_one(n: &usize) -> bool {
    *n == 1
}
fn is_false(b: &bool) -> bool {
    !*b
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            text: String::new(),
            colspan: 1,
            rowspan: 1,
            bold: false,
            italic: false,
            underline: false,
            align: String::new(),
            fill: String::new(),
        }
    }
}

impl Cell {
    pub fn with_text(text: &str) -> Cell {
        Cell { text: text.to_string(), ..Cell::default() }
    }
}

/// A whole table: its headings, its data, and its own stylesheet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Table {
    /// Which `wg-tN` this is, so its CSS can be scoped to it.
    pub id: u32,
    /// `thead` rows. Usually one; more is legal and occasionally wanted.
    #[serde(default)]
    pub head: Vec<Vec<Cell>>,
    #[serde(default)]
    pub body: Vec<Vec<Cell>>,
    #[serde(default)]
    pub foot: Vec<Vec<Cell>>,
    /// Column widths in millimetres, one per grid column, empty when the table has no opinion.
    ///
    /// Imported documents DO have an opinion — `w:tblGrid` states every column's width — and
    /// discarding it was why a converted form wrapped "Student Name" onto two lines where Word
    /// and LibreOffice fit it on one, which then compounded down the page (Piers, 2026-08-06).
    /// Rendered as a `<colgroup>` plus scoped rules, never inline styles.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cols: Vec<f64>,

    /// selector (as written in [`SELECTORS`]) → its style.
    ///
    /// The same [`TagStyle`] the document style sidebar uses, so there is one style vocabulary in
    /// the app rather than two that drift.
    #[serde(default)]
    pub css: BTreeMap<String, TagStyle>,
}

impl Table {
    /// A new table of `rows` body rows and `cols` columns, with a heading row.
    pub fn new(id: u32, rows: usize, cols: usize) -> Table {
        let cols = cols.max(1);
        let rows = rows.max(1);
        let head = vec![(1..=cols).map(|c| Cell::with_text(&format!("Heading {c}"))).collect()];
        let body = (0..rows).map(|_| (0..cols).map(|_| Cell::default()).collect()).collect();
        let mut css = BTreeMap::new();
        // A default that makes it look like a table rather than a list of words. Everything else is
        // left alone, so the document's own style still shows through.
        let cell = TagStyle {
            border: "1px solid #999999".into(),
            padding: "6px".into(),
            ..TagStyle::default()
        };
        css.insert("th".to_string(), TagStyle { font_weight: "bold".into(), ..cell.clone() });
        css.insert("td".to_string(), cell);
        Table { id, head, body, foot: Vec::new(), cols: Vec::new(), css }
    }

    pub fn class(&self) -> String {
        format!("{TABLE_CLASS_PREFIX}{}", self.id)
    }

    /// How many columns the table has, counting spans.
    ///
    /// Taken from the widest **grid**, not the longest row: a row whose cells carry `colspan` has
    /// fewer cells than columns, and a row sitting under a `rowspan` has fewer still.
    pub fn columns(&self) -> usize {
        let mut width = 0;
        for section in [&self.head, &self.body, &self.foot] {
            let grid = Grid::of(section);
            width = width.max(grid.width);
        }
        width.max(1)
    }

    // ---- structural edits ------------------------------------------------------------------

    /// Add a body row at `at` (clamped), matching the current column count.
    pub fn insert_row(&mut self, at: usize) {
        let cols = self.columns();
        let at = at.min(self.body.len());
        // A row cannot be inserted through a vertical span: shorten any span that would cross it.
        let grid = Grid::of(&self.body);
        for r in 0..at {
            for cell in grid.anchors_in_row(r) {
                if cell.row + cell.rowspan > at {
                    self.body[cell.row][cell.index].rowspan = at - cell.row;
                }
            }
        }
        self.body.insert(at, (0..cols).map(|_| Cell::default()).collect());
    }

    pub fn delete_row(&mut self, at: usize) {
        if self.body.len() <= 1 || at >= self.body.len() {
            return;
        }
        // Any span reaching into the doomed row gets one shorter.
        let grid = Grid::of(&self.body);
        let shrink: Vec<(usize, usize)> = grid
            .cells
            .iter()
            .filter(|c| c.row < at && c.row + c.rowspan > at)
            .map(|c| (c.row, c.index))
            .collect();
        for (row, index) in shrink {
            self.body[row][index].rowspan -= 1;
        }
        self.body.remove(at);
    }

    /// Add a column at `at` in every section.
    pub fn insert_column(&mut self, at: usize) {
        let cols = self.columns();
        let at = at.min(cols);
        for section in [&mut self.head, &mut self.body, &mut self.foot] {
            insert_column_in(section, at);
        }
    }

    pub fn delete_column(&mut self, at: usize) {
        if self.columns() <= 1 {
            return;
        }
        for section in [&mut self.head, &mut self.body, &mut self.foot] {
            delete_column_in(section, at);
        }
    }

    /// Merge the rectangle from `(r0, c0)` to `(r1, c1)` in the body into one cell.
    ///
    /// The anchor keeps its text and grows; the cells it swallows are removed, and their text is
    /// appended to the anchor's rather than thrown away — losing a cell's words to a layout
    /// operation is the kind of thing you only notice after saving.
    pub fn merge(&mut self, r0: usize, c0: usize, r1: usize, c1: usize) -> bool {
        merge_in(&mut self.body, r0, c0, r1, c1)
    }

    /// Undo a merge: the cell goes back to 1×1 and the positions it covered become empty cells.
    pub fn split(&mut self, row: usize, col: usize) -> bool {
        split_in(&mut self.body, row, col)
    }

    // ---- rendering -------------------------------------------------------------------------

    /// The scoped stylesheet for this table.
    pub fn css_text(&self) -> String {
        let class = self.class();
        let mut out = String::new();
        for (selector, _) in SELECTORS {
            let mut declarations: Vec<(&str, String)> = Vec::new();
            // `border-collapse` is not a knob: a document table with separated borders looks like a
            // mistake, and there is no reason to offer the mistake.
            if *selector == "table" {
                declarations.push(("border-collapse", "collapse".to_string()));
            }
            if let Some(style) = self.css.get(*selector) {
                for (property, value) in style.declarations() {
                    declarations.push((property, value.to_string()));
                }
            }
            if declarations.is_empty() {
                continue;
            }
            let body = declarations
                .iter()
                .map(|(p, v)| format!("{p}: {v};"))
                .collect::<Vec<_>>()
                .join(" ");
            out.push_str(&format!("{} {{ {body} }}\n", scope(selector, &class)));
        }
        // Column widths, and the layout mode that makes them authoritative. `table-layout: fixed`
        // is the difference between "a hint the browser may ignore when text is long" and "the
        // width Word said" — without it a long word silently widens its column and every other
        // column pays, which is exactly the compounding drift converted forms suffered.
        if !self.cols.is_empty() {
            let total: f64 = self.cols.iter().sum();
            if total > 0.0 {
                out.push_str(&format!(
                    "table.{class} {{ table-layout: fixed; width: {total:.1}mm; }}\n"
                ));
                for (i, w) in self.cols.iter().enumerate() {
                    out.push_str(&format!(".{class} col.wg-c{} {{ width: {w:.1}mm; }}\n", i + 1));
                }
            }
        }

        // Per-cell fills, as `.cell_rN_cM` rules — the deviation list. Sheet rules, never inline:
        // the point is that reading this block tells you exactly which cells differ and how.
        let positions = self.cell_positions();
        for (si, section) in [&self.head, &self.body, &self.foot].into_iter().enumerate() {
            for (ri, row) in section.iter().enumerate() {
                for (ci, cell) in row.iter().enumerate() {
                    if cell.fill.is_empty() {
                        continue;
                    }
                    if let Some(&(r, c)) = positions[si].get(ri).and_then(|cols| cols.get(ci)) {
                        out.push_str(&format!(
                            ".{class} .cell_r{r}_c{c} {{ background: {}; }}\n",
                            cell.fill
                        ));
                    }
                }
            }
        }
        out
    }

    /// Visual `(row, col)` of every cell, 1-based, spans resolved, numbered continuously across
    /// head → body → foot. The single source of the `.cell_rN_cM` numbering: the renderer and
    /// [`Self::css_text`] must agree on it or the class and its rule name different cells.
    fn cell_positions(&self) -> [Vec<Vec<(usize, usize)>>; 3] {
        let mut occupied: Vec<Vec<bool>> = Vec::new();
        let mut global_row = 0usize;
        let mut out: [Vec<Vec<(usize, usize)>>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for (si, section) in [&self.head, &self.body, &self.foot].into_iter().enumerate() {
            for row in section.iter() {
                if occupied.len() <= global_row {
                    occupied.resize(global_row + 1, Vec::new());
                }
                let mut cols = Vec::new();
                let mut col = 0usize;
                for cell in row {
                    while occupied[global_row].get(col).copied().unwrap_or(false) {
                        col += 1;
                    }
                    cols.push((global_row + 1, col + 1));
                    for r in global_row..global_row + cell.rowspan {
                        if occupied.len() <= r {
                            occupied.resize(r + 1, Vec::new());
                        }
                        if occupied[r].len() < col + cell.colspan {
                            occupied[r].resize(col + cell.colspan, false);
                        }
                        for c in col..col + cell.colspan {
                            occupied[r][c] = true;
                        }
                    }
                    col += cell.colspan;
                }
                out[si].push(cols);
                global_row += 1;
            }
        }
        out
    }

    /// The whole block: markers, style, table.
    pub fn to_block(&self) -> String {
        let mut out = String::new();
        out.push_str(START_MARKER);
        out.push('\n');
        let css = self.css_text();
        if !css.trim().is_empty() {
            out.push_str("<style>\n");
            out.push_str(&css);
            out.push_str("</style>\n");
        }
        out.push_str(&format!(
            "<table class=\"{}\" {DATA_ATTR}=\"{}\">\n",
            self.class(),
            escape_attr(&self.to_json())
        ));
        // Column widths as a colgroup: the <col> elements carry only classes, the widths live in
        // the scoped style block with everything else.
        if !self.cols.is_empty() {
            out.push_str("  <colgroup>\n");
            for i in 0..self.cols.len() {
                out.push_str(&format!("    <col class=\"wg-c{}\">\n", i + 1));
            }
            out.push_str("  </colgroup>\n");
        }
        let positions = self.cell_positions();
        render_section(&mut out, "thead", "th", &self.head, &positions[0]);
        render_section(&mut out, "tbody", "td", &self.body, &positions[1]);
        render_section(&mut out, "tfoot", "td", &self.foot, &positions[2]);
        out.push_str("</table>\n");
        out.push_str(END_MARKER);
        out.push('\n');
        out
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    pub fn from_json(json: &str) -> Option<Table> {
        serde_json::from_str(json).ok()
    }
}

/// `thead tr` scoped to one table becomes `.wg-t1 thead tr`; the table itself becomes `table.wg-t1`,
/// which is more specific than a bare `table` rule elsewhere in the document.
fn scope(selector: &str, class: &str) -> String {
    if selector == "table" {
        format!("table.{class}")
    } else {
        format!(".{class} {selector}")
    }
}

fn render_section(
    out: &mut String,
    section: &str,
    cell_tag: &str,
    rows: &[Vec<Cell>],
    positions: &[Vec<(usize, usize)>],
) {
    if rows.is_empty() {
        return;
    }
    out.push_str(&format!("  <{section}>\n"));
    for (ri, row) in rows.iter().enumerate() {
        out.push_str("    <tr>\n");
        for (ci, cell) in row.iter().enumerate() {
            let mut attrs = String::new();
            if cell.colspan > 1 {
                attrs.push_str(&format!(" colspan=\"{}\"", cell.colspan));
            }
            if cell.rowspan > 1 {
                attrs.push_str(&format!(" rowspan=\"{}\"", cell.rowspan));
            }
            // Classes accumulate: alignment and the per-cell fill hook may coexist.
            let mut classes: Vec<String> = Vec::new();
            if !cell.align.is_empty() {
                classes.push(format!("wg-a-{}", cell.align));
            }
            if !cell.fill.is_empty() {
                if let Some(&(r, c)) = positions.get(ri).and_then(|cols| cols.get(ci)) {
                    classes.push(format!("cell_r{r}_c{c}"));
                }
            }
            if !classes.is_empty() {
                attrs.push_str(&format!(" class=\"{}\"", classes.join(" ")));
            }
            let mut text = escape_cell(&cell.text);
            if text.is_empty() {
                text.push_str("<br>");
            }
            // Emphasis is markup, not style: a bold heading is bold in a browser that loads no
            // stylesheet at all, and it survives being pasted somewhere else.
            if cell.underline {
                text = format!("<u>{text}</u>");
            }
            if cell.italic {
                text = format!("<em>{text}</em>");
            }
            if cell.bold {
                text = format!("<strong>{text}</strong>");
            }
            out.push_str(&format!("      <{cell_tag}{attrs}>{text}</{cell_tag}>\n"));
        }
        out.push_str("    </tr>\n");
    }
    out.push_str(&format!("  </{section}>\n"));
}

/// Escape cell text and keep runs of spaces visible — the same rule the docx converter applies,
/// and it must live here too because a table adopted into the MODEL renders through this path,
/// not through the converter's markup (2026-08-06: the pen-fill date fields still collapsed).
pub fn escape_cell(s: &str) -> String {
    let escaped = escape(s);
    let chars: Vec<char> = escaped.chars().collect();
    let mut out = String::with_capacity(escaped.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ' ' {
            let start = i;
            while i < chars.len() && chars[i] == ' ' {
                i += 1;
            }
            let run = i - start;
            if run >= 2 {
                for _ in 0..run - 1 {
                    out.push_str("&nbsp;");
                }
            }
            out.push(' ');
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn escape_attr(s: &str) -> String {
    escape(s).replace('"', "&quot;")
}

/// Undo [`escape_attr`], for reading the model back out of the attribute.
pub fn unescape_attr(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&gt;", ">").replace("&lt;", "<").replace("&amp;", "&")
}

// ---- talking to the document ---------------------------------------------------------------
//
// A table block is delimited by comments, which are DOM nodes like any other. Everything here works
// in terms of a Range from the opening comment to the closing one, so a save replaces the whole
// block in one operation rather than trying to patch markup in place.

/// The model of the table the caret is inside, or `''`.
pub fn find_at_cursor_script() -> String {
    format!(
        "(function () {{
           const sel = window.getSelection();
           let node = sel && sel.anchorNode;
           if (!node && document.activeElement) node = document.activeElement;
           let el = node ? (node.nodeType === 1 ? node : node.parentElement) : null;
           while (el && el.tagName !== 'TABLE') el = el.parentElement;
           if (!el || !el.hasAttribute('{attr}')) return '';
           return el.getAttribute('{attr}');
         }})()",
        attr = DATA_ATTR,
    )
}

/// The table AND the cell the caret sits in: `"section|row|index|json"`, empty when the caret is
/// not in a table. `section` is `head`/`body`/`foot`, `row` is the row's index within that
/// section and `index` its position in that row's array — the same coordinates the model uses,
/// because the renderer emits cells in model order.
///
/// This is what makes table editing happen IN the document (Piers, 2026-08-06) rather than in the
/// table window: the caret is the selection, so no dialog has to be opened to say where.
pub fn find_cell_at_cursor_script() -> String {
    format!(
        "(function () {{
           const sel = window.getSelection();
           let node = sel && sel.anchorNode;
           if (!node && document.activeElement) node = document.activeElement;
           let el = node ? (node.nodeType === 1 ? node : node.parentElement) : null;
           let cell = el;
           while (cell && cell.tagName !== 'TD' && cell.tagName !== 'TH') cell = cell.parentElement;
           let table = cell || el;
           while (table && table.tagName !== 'TABLE') table = table.parentElement;
           if (!table || !table.hasAttribute('{attr}')) return '';
           const json = table.getAttribute('{attr}');
           if (!cell) return 'body|0|0|' + json;
           const tr = cell.parentElement;
           const sectionTag = tr.parentElement ? tr.parentElement.tagName : 'TBODY';
           const section = sectionTag === 'THEAD' ? 'head' : (sectionTag === 'TFOOT' ? 'foot' : 'body');
           const rows = Array.prototype.slice.call(tr.parentElement.children);
           const row = rows.indexOf(tr);
           const index = Array.prototype.slice.call(tr.children).indexOf(cell);
           return section + '|' + row + '|' + index + '|' + json;
         }})()",
        attr = DATA_ATTR,
    )
}

/// The highest `wg-tN` already in the document, so a new table cannot collide with one whose block
/// was deleted and re-added.
pub fn highest_id_script() -> String {
    format!(
        "(function () {{
           let max = 0;
           document.querySelectorAll('table[class]').forEach(function (t) {{
             t.classList.forEach(function (c) {{
               const m = /^{prefix}(\\d+)$/.exec(c);
               if (m) max = Math.max(max, parseInt(m[1], 10));
             }});
           }});
           return String(max);
         }})()",
        prefix = TABLE_CLASS_PREFIX,
    )
}

/// Replace an existing block with freshly generated markup, or remove it when `html` is empty.
///
/// The range runs from the opening comment to the closing one, so the `<style>` that sits between
/// the comment and the table goes with it. If either marker is missing — a document somebody has
/// edited by hand — the table element itself is the boundary, which still leaves the document valid.
pub fn replace_block_script(class: &str, html: &str) -> String {
    format!(
        "(function (cls, html) {{
           const table = document.querySelector('table.' + cls);
           if (!table) return '';
           let start = table, end = table;
           for (let n = table.previousSibling; n; n = n.previousSibling) {{
             if (n.nodeType === 8 && n.data.indexOf('table block') !== -1
                 && n.data.indexOf('END') === -1) {{ start = n; break; }}
           }}
           for (let n = table.nextSibling; n; n = n.nextSibling) {{
             if (n.nodeType === 8 && n.data.indexOf('END table block') !== -1) {{ end = n; break; }}
           }}
           const range = document.createRange();
           range.setStartBefore(start);
           range.setEndAfter(end);
           range.deleteContents();
           if (html) range.insertNode(range.createContextualFragment(html));
           return '1';
         }})({cls}, {html})",
        cls = crate::js::string(class),
        html = crate::js::string(html),
    )
}

/// Put a new block in at the caret.
///
/// A Range rather than `execCommand('insertHTML')`: WebKit's insertHTML sanitises what it is given
/// and drops the boundary comments, which are the whole mechanism for finding the block again.
pub fn insert_block_script(html: &str) -> String {
    format!(
        "(function (html) {{
           const sel = window.getSelection();
           let range;
           if (sel && sel.rangeCount > 0) {{
             range = sel.getRangeAt(0);
             range.deleteContents();
           }} else {{
             range = document.createRange();
             range.selectNodeContents(document.body);
             range.collapse(false);
           }}
           /* A table cannot live inside a paragraph, and inserting at the caret would cut the
              paragraph in half — measured: a paragraph reading Before the table. became Befo,
              with the rest stranded underneath. So climb to the top-level block the caret is in
              and put the table after it, which is where a person pointing at that paragraph
              means it to go. */
           let host = range.startContainer;
           if (host.nodeType !== 1) host = host.parentElement;
           while (host && host.parentElement && host.parentElement !== document.body) {{
             host = host.parentElement;
           }}
           if (host && host.parentElement === document.body) {{
             range = document.createRange();
             range.setStartAfter(host);
             range.collapse(true);
           }}
           range.insertNode(range.createContextualFragment(html));
           return '1';
         }})({html})",
        html = crate::js::string(html),
    )
}

// ---- the grid ------------------------------------------------------------------------------
//
// The row arrays hold only *anchor* cells; a merged cell occupies positions that no array entry
// corresponds to. Everything structural needs to know where those positions are, so it is worked
// out once, here, rather than guessed at in five places.

/// One anchor cell, and where it actually sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    /// Row in the section.
    pub row: usize,
    /// Index within that row's array.
    pub index: usize,
    /// Grid column the cell starts at.
    pub col: usize,
    pub colspan: usize,
    pub rowspan: usize,
}

pub struct Grid {
    pub cells: Vec<Placed>,
    pub width: usize,
}

impl Grid {
    /// Lay a section out, resolving every span into a grid position.
    pub fn of(rows: &[Vec<Cell>]) -> Grid {
        let mut occupied: Vec<Vec<bool>> = Vec::new();
        let mut cells = Vec::new();
        let mut width = 0usize;

        for (r, row) in rows.iter().enumerate() {
            while occupied.len() <= r {
                occupied.push(Vec::new());
            }
            let mut col = 0usize;
            for (index, cell) in row.iter().enumerate() {
                // Step over anything a cell above has already claimed.
                while *occupied[r].get(col).unwrap_or(&false) {
                    col += 1;
                }
                let colspan = cell.colspan.max(1);
                let rowspan = cell.rowspan.max(1);
                for dr in 0..rowspan {
                    while occupied.len() <= r + dr {
                        occupied.push(Vec::new());
                    }
                    for dc in 0..colspan {
                        let (rr, cc) = (r + dr, col + dc);
                        if occupied[rr].len() <= cc {
                            occupied[rr].resize(cc + 1, false);
                        }
                        occupied[rr][cc] = true;
                    }
                }
                cells.push(Placed { row: r, index, col, colspan, rowspan });
                col += colspan;
                width = width.max(col);
            }
        }
        Grid { cells, width }
    }

    pub fn anchors_in_row(&self, row: usize) -> Vec<Placed> {
        self.cells.iter().copied().filter(|c| c.row == row).collect()
    }

    /// The anchor covering grid position `(row, col)`, if any.
    pub fn at(&self, row: usize, col: usize) -> Option<Placed> {
        self.cells.iter().copied().find(|c| {
            row >= c.row && row < c.row + c.rowspan && col >= c.col && col < c.col + c.colspan
        })
    }
}

fn insert_column_in(rows: &mut Vec<Vec<Cell>>, at: usize) {
    let grid = Grid::of(rows);
    // Walk rows in reverse so earlier indices stay valid as we splice.
    for r in (0..rows.len()).rev() {
        // A cell straddling the insertion point widens instead of a new cell appearing.
        if let Some(anchor) = grid.at(r, at.saturating_sub(1)) {
            if at > 0 && anchor.col < at && anchor.col + anchor.colspan > at {
                if anchor.row == r {
                    rows[anchor.row][anchor.index].colspan += 1;
                }
                continue;
            }
        }
        // Where in this row's array does grid column `at` fall?
        let index = grid
            .anchors_in_row(r)
            .into_iter()
            .filter(|c| c.col < at)
            .count();
        // A row entirely covered by a rowspan from above gets nothing.
        if grid.at(r, at).map(|c| c.row != r).unwrap_or(false) {
            continue;
        }
        let at_index = index.min(rows[r].len());
        rows[r].insert(at_index, Cell::default());
    }
}

fn delete_column_in(rows: &mut Vec<Vec<Cell>>, at: usize) {
    let grid = Grid::of(rows);
    let mut narrow: Vec<(usize, usize)> = Vec::new();
    let mut remove: Vec<(usize, usize)> = Vec::new();
    for r in 0..rows.len() {
        let Some(anchor) = grid.at(r, at) else { continue };
        if anchor.row != r {
            continue; // owned by a row above; handled there
        }
        if anchor.colspan > 1 {
            narrow.push((anchor.row, anchor.index));
        } else {
            remove.push((anchor.row, anchor.index));
        }
    }
    for (row, index) in narrow {
        rows[row][index].colspan -= 1;
    }
    // Remove back-to-front so indices stay valid.
    remove.sort_by(|a, b| b.cmp(a));
    for (row, index) in remove {
        if rows[row].len() > 1 {
            rows[row].remove(index);
        }
    }
}

fn merge_in(rows: &mut Vec<Vec<Cell>>, r0: usize, c0: usize, r1: usize, c1: usize) -> bool {
    let (r0, r1) = (r0.min(r1), r0.max(r1));
    let (c0, c1) = (c0.min(c1), c0.max(c1));
    let grid = Grid::of(rows);
    let Some(anchor) = grid.at(r0, c0) else { return false };
    if anchor.row != r0 || anchor.col != c0 {
        return false;
    }
    if r1 >= rows.len() || (r0 == r1 && c0 == c1) {
        return false;
    }

    // Everything the rectangle covers, other than the anchor itself.
    let mut swallowed: Vec<Placed> = Vec::new();
    for r in r0..=r1 {
        for c in c0..=c1 {
            if let Some(cell) = grid.at(r, c) {
                if (cell.row, cell.index) != (anchor.row, anchor.index)
                    && !swallowed.iter().any(|s| (s.row, s.index) == (cell.row, cell.index))
                {
                    // Refuse a merge that would only half-swallow a cell — the result would be a
                    // ragged table, and silently reshaping someone's data is worse than declining.
                    if cell.row < r0
                        || cell.row + cell.rowspan > r1 + 1
                        || cell.col < c0
                        || cell.col + cell.colspan > c1 + 1
                    {
                        return false;
                    }
                    swallowed.push(cell);
                }
            }
        }
    }

    // Keep the words. A merge is a layout operation and must not lose text.
    let mut text = rows[anchor.row][anchor.index].text.clone();
    for cell in &swallowed {
        let extra = rows[cell.row][cell.index].text.trim().to_string();
        if !extra.is_empty() {
            if !text.trim().is_empty() {
                text.push(' ');
            }
            text.push_str(&extra);
        }
    }
    rows[anchor.row][anchor.index].text = text;
    rows[anchor.row][anchor.index].colspan = c1 - c0 + 1;
    rows[anchor.row][anchor.index].rowspan = r1 - r0 + 1;

    let mut to_remove: Vec<(usize, usize)> = swallowed.iter().map(|c| (c.row, c.index)).collect();
    to_remove.sort_by(|a, b| b.cmp(a));
    for (row, index) in to_remove {
        rows[row].remove(index);
    }
    true
}

fn split_in(rows: &mut Vec<Vec<Cell>>, row: usize, col: usize) -> bool {
    let grid = Grid::of(rows);
    let Some(anchor) = grid.at(row, col) else { return false };
    if anchor.colspan == 1 && anchor.rowspan == 1 {
        return false;
    }
    let (r0, c0) = (anchor.row, anchor.col);
    let (colspan, rowspan) = (anchor.colspan, anchor.rowspan);
    rows[anchor.row][anchor.index].colspan = 1;
    rows[anchor.row][anchor.index].rowspan = 1;

    // Put empty cells back where the merge had swallowed them, one row at a time so the array
    // index for each insertion is worked out against the layout as it now stands.
    for dr in 0..rowspan {
        for dc in 0..colspan {
            if dr == 0 && dc == 0 {
                continue;
            }
            let (r, c) = (r0 + dr, c0 + dc);
            if r >= rows.len() {
                continue;
            }
            let now = Grid::of(rows);
            if now.at(r, c).is_some() {
                continue;
            }
            let index = now.anchors_in_row(r).into_iter().filter(|x| x.col < c).count();
            let at_index = index.min(rows[r].len());
        rows[r].insert(at_index, Cell::default());
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(rows: usize, cols: usize) -> Table {
        Table::new(1, rows, cols)
    }

    #[test]
    fn a_new_table_has_a_heading_row_and_the_asked_for_shape() {
        let table = t(2, 3);
        assert_eq!(table.head.len(), 1);
        assert_eq!(table.head[0].len(), 3);
        assert_eq!(table.body.len(), 2);
        assert_eq!(table.columns(), 3);
    }

    #[test]
    fn the_block_is_the_shape_the_document_expects() {
        let block = t(2, 1).to_block();
        assert!(block.starts_with(START_MARKER), "{block}");
        assert!(block.trim_end().ends_with(END_MARKER), "{block}");
        assert!(block.contains("<thead>") && block.contains("<tbody>"), "{block}");
        assert!(block.contains("<th") && block.contains("<td"), "{block}");
        assert!(block.contains("class=\"wg-t1\""), "{block}");
    }

    #[test]
    fn the_css_is_scoped_to_this_table_and_not_to_every_table() {
        // The style block lives in the document, so an unscoped `table { … }` would restyle the lot.
        let mut table = t(1, 1);
        table.css.insert(
            "tbody tr:nth-child(odd)".into(),
            TagStyle { background: "#f4f4f4".into(), ..TagStyle::default() },
        );
        let css = table.css_text();
        assert!(css.contains("table.wg-t1 {"), "{css}");
        assert!(css.contains(".wg-t1 tbody tr:nth-child(odd) { background: #f4f4f4; }"), "{css}");
        assert!(!css.lines().any(|l| l.trim_start().starts_with("table {")), "{css}");
    }

    #[test]
    fn the_model_round_trips_through_the_data_attribute() {
        let mut table = t(2, 2);
        table.body[0][0] = Cell { text: "a & b < c".into(), bold: true, ..Cell::default() };
        table.merge(0, 0, 0, 1);
        let json = table.to_json();
        let attr = escape_attr(&json);
        // The attribute is what actually lands in the file, quotes and all.
        assert!(!attr.contains('"'), "an unescaped quote would end the attribute: {attr}");
        assert_eq!(Table::from_json(&unescape_attr(&attr)), Some(table));
    }

    #[test]
    fn a_cell_containing_the_comment_terminator_does_not_end_the_block() {
        // The reason the model is in an attribute rather than in the opening comment.
        let mut table = t(1, 1);
        table.body[0][0] = Cell::with_text("look --> here");
        let block = table.to_block();
        assert_eq!(block.matches("-->").count(), 2, "only the two markers close:\n{block}");
        let json = block
            .split(&format!("{DATA_ATTR}=\""))
            .nth(1)
            .and_then(|s| s.split('"').next())
            .expect("attribute present");
        assert_eq!(Table::from_json(&unescape_attr(json)).unwrap().body[0][0].text, "look --> here");
    }

    #[test]
    fn emphasis_is_markup_so_it_survives_without_the_stylesheet() {
        let mut table = t(1, 1);
        table.body[0][0] = Cell {
            text: "x".into(),
            bold: true,
            italic: true,
            underline: true,
            ..Cell::default()
        };
        let block = table.to_block();
        assert!(block.contains("<strong><em><u>x</u></em></strong>"), "{block}");
    }

    // ---- the grid, which is the part that is painful to get right -------------------------

    #[test]
    fn a_merged_cell_makes_the_row_shorter_than_the_table_is_wide() {
        let mut table = t(2, 3);
        assert!(table.merge(0, 0, 0, 1));
        assert_eq!(table.body[0].len(), 2, "two array entries");
        assert_eq!(table.columns(), 3, "but still three columns");
        assert_eq!(table.body[0][0].colspan, 2);
    }

    #[test]
    fn a_vertical_merge_leaves_the_row_below_with_fewer_cells() {
        let mut table = t(3, 2);
        assert!(table.merge(0, 0, 1, 0));
        assert_eq!(table.body[0][0].rowspan, 2);
        assert_eq!(table.body[1].len(), 1, "the covered position has no array entry");
        let grid = Grid::of(&table.body);
        assert_eq!(grid.at(1, 0).map(|c| c.row), Some(0), "row 1 col 0 belongs to the cell above");
        assert_eq!(grid.at(1, 1).map(|c| c.row), Some(1));
    }

    #[test]
    fn merging_keeps_the_words_from_every_cell_it_swallows() {
        let mut table = t(1, 3);
        table.body[0][0] = Cell::with_text("one");
        table.body[0][1] = Cell::with_text("two");
        table.body[0][2] = Cell::with_text("three");
        assert!(table.merge(0, 0, 0, 2));
        assert_eq!(table.body[0][0].text, "one two three", "a layout change must not lose text");
    }

    #[test]
    fn splitting_puts_the_covered_positions_back() {
        let mut table = t(3, 3);
        assert!(table.merge(0, 1, 1, 2));
        assert_eq!(table.body[0].len(), 2);
        assert!(table.split(0, 1));
        assert_eq!(table.body[0].len(), 3);
        assert_eq!(table.body[1].len(), 3);
        assert_eq!(table.columns(), 3);
        let grid = Grid::of(&table.body);
        assert_eq!(grid.cells.len(), 9, "a 3x3 grid has nine cells again");
    }

    #[test]
    fn a_merge_that_would_only_half_swallow_a_cell_is_refused() {
        let mut table = t(2, 3);
        assert!(table.merge(0, 1, 0, 2));
        // Now try to merge a rectangle that cuts that cell in half.
        assert!(!table.merge(0, 0, 0, 1), "declining beats silently reshaping the data");
        assert_eq!(table.body[0].len(), 2, "unchanged");
    }

    #[test]
    fn inserting_a_column_through_a_span_widens_it_rather_than_ragging_the_table() {
        let mut table = t(2, 3);
        table.merge(0, 0, 0, 1);
        table.insert_column(1);
        assert_eq!(table.columns(), 4);
        assert_eq!(table.body[0][0].colspan, 3, "the span grew to cover the new column");
        assert_eq!(table.body[1].len(), 4);
    }

    #[test]
    fn deleting_a_column_narrows_a_span_rather_than_removing_the_cell() {
        let mut table = t(2, 3);
        table.merge(0, 0, 0, 1);
        table.delete_column(0);
        assert_eq!(table.columns(), 2);
        assert_eq!(table.body[0][0].colspan, 1);
        assert_eq!(table.body[1].len(), 2);
    }

    #[test]
    fn deleting_a_row_shortens_a_span_reaching_into_it() {
        let mut table = t(3, 2);
        table.merge(0, 0, 1, 0);
        assert_eq!(table.body[0][0].rowspan, 2);
        table.delete_row(1);
        assert_eq!(table.body.len(), 2);
        assert_eq!(table.body[0][0].rowspan, 1, "the span cannot outlive the row it covered");
        let grid = Grid::of(&table.body);
        assert_eq!(grid.width, 2);
    }

    #[test]
    fn the_last_row_and_column_cannot_be_deleted_away() {
        let mut table = t(1, 1);
        table.delete_row(0);
        table.delete_column(0);
        assert_eq!(table.body.len(), 1);
        assert_eq!(table.columns(), 1);
    }

    #[test]
    fn every_section_keeps_the_same_column_count_when_one_is_added() {
        let mut table = t(2, 2);
        table.foot = vec![vec![Cell::with_text("total"), Cell::default()]];
        table.insert_column(2);
        assert_eq!(table.head[0].len(), 3);
        assert_eq!(table.body[0].len(), 3);
        assert_eq!(table.foot[0].len(), 3);
    }

    #[test]
    fn a_filled_cell_gets_a_cell_class_and_a_sheet_rule_never_an_inline_style() {
        let mut table = t(2, 2);
        table.body[1][1].fill = "#d9d9d9".into();
        let block = table.to_block();
        // Head row is row 1, so body row 2 is visual row 3; second column is c2.
        assert!(block.contains("class=\"cell_r3_c2\""), "{block}");
        assert!(block.contains(&format!(".{} .cell_r3_c2 {{ background: #d9d9d9; }}", table.class())), "{block}");
        assert!(!block.contains("style=\""), "no inline styles ever: {block}");
        // And it survives the JSON round trip that regeneration relies on.
        let back = Table::from_json(&table.to_json()).unwrap();
        assert_eq!(back.body[1][1].fill, "#d9d9d9");
    }

    #[test]
    fn fill_numbering_respects_spans_above_and_beside() {
        // Body-only table: r1c1 spans 2 rows, so the second row's first LISTED cell sits at c2;
        // give it a fill and the class must say r2_c2, not r2_c1.
        let mut table = Table { id: 9, cols: Vec::new(), head: Vec::new(), body: vec![
            vec![Cell { rowspan: 2, ..Cell::with_text("tall") }, Cell::with_text("a")],
            vec![Cell { fill: "#eee".into(), ..Cell::with_text("shifted") }],
        ], foot: Vec::new(), css: Default::default() };
        table.css.clear();
        let block = table.to_block();
        assert!(block.contains("class=\"cell_r2_c2\""), "{block}");
        assert!(block.contains(".wg-t9 .cell_r2_c2 { background: #eee; }"), "{block}");
    }
}
