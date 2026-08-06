//! Read a WebGen Word document's HTML into the export model.
//!
//! This is a reader for OUR OWN output, not a general HTML parser: the document was written by
//! this app (or converted by this crate), so the shapes are known — `<p>`, `<h1>`-`<h6>`,
//! `<ul>/<ol>/<li>`, `<table>` with an optional `<colgroup>`, `<strong>/<em>/<u>/<s>`, `<a href>`,
//! `<img src>`, and the page-break `<hr>`. Anything unrecognised contributes its text and nothing
//! else, which is the right failure: a paragraph of prose is never lost because it was wrapped in
//! a tag this does not model.
//!
//! Column widths come back from the scoped `col.wg-cN { width: …mm }` rules the table block
//! writes, so the widths a document imported from Word survive the trip back out.

use crate::to_docx::{CellOut, Node, Run, TableOut};
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::HashMap;

/// What the reader found.
pub struct Parsed {
    pub nodes: Vec<Node>,
    /// Image `src` values in document order — the caller resolves them to bytes.
    pub images: Vec<String>,
}

#[derive(Clone, Default)]
struct Fmt {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    link: Option<String>,
}

/// Parse a full document (or a body fragment) into export nodes.
pub fn parse(html: &str) -> Parsed {
    let widths = column_widths(html);
    let mut r = Reader::from_str(html);
    r.config_mut().trim_text(false);
    r.config_mut().check_end_names = false;
    let mut buf = Vec::new();

    let mut nodes: Vec<Node> = Vec::new();
    let mut images: Vec<String> = Vec::new();
    // Where runs currently accumulate: the document, or a cell.
    let mut runs: Vec<Run> = Vec::new();
    let mut fmt = Fmt::default();
    let mut list_depth: i32 = -1;
    let mut ordered: Vec<bool> = Vec::new();
    let mut heading: u8 = 0;
    let mut align: Option<String> = None;
    let mut in_body = !html.contains("<body");
    let mut in_style = false;

    // Table state. Only one level of nesting is modelled; a table inside a cell is flattened to
    // its text, which is what OOXML would need a separate table part for anyway.
    struct TblState {
        rows: Vec<Vec<CellOut>>,
        row: Vec<CellOut>,
        cell: Vec<Node>,
        colspan: u32,
        rowspan: u32,
        fill: Option<String>,
        header: bool,
        in_cell: bool,
        key: String,
    }
    let mut tbl: Option<TblState> = None;

    let flush_para = |runs: &mut Vec<Run>,
                      heading: &mut u8,
                      align: &mut Option<String>,
                      list_depth: i32,
                      ordered: &[bool],
                      out: &mut Vec<Node>| {
        let taken: Vec<Run> = runs.drain(..).filter(|r| !r.text.is_empty() || r.image.is_some()).collect();
        if taken.is_empty() {
            *heading = 0;
            *align = None;
            return;
        }
        if list_depth >= 0 {
            out.push(Node::Item {
                ordered: *ordered.last().unwrap_or(&false),
                depth: list_depth.min(8) as u8,
                runs: taken,
            });
        } else {
            out.push(Node::Para { level: *heading, runs: taken, align: align.clone() });
        }
        *heading = 0;
        *align = None;
    };

    loop {
        let ev = match r.read_event_into(&mut buf) {
            Ok(e) => e,
            Err(_) => break,
        };
        match ev {
            Event::Start(e) | Event::Empty(e) => {
                let name = e.name().as_ref().to_ascii_lowercase();
                let attrs: HashMap<String, String> = e
                    .attributes()
                    .flatten()
                    .map(|a| {
                        (
                            String::from_utf8_lossy(a.key.as_ref()).to_ascii_lowercase(),
                            a.unescape_value().map(|v| v.to_string()).unwrap_or_default(),
                        )
                    })
                    .collect();
                match name.as_slice() {
                    b"body" => in_body = true,
                    b"style" | b"script" | b"head" => in_style = true,
                    b"strong" | b"b" => fmt.bold = true,
                    b"em" | b"i" => fmt.italic = true,
                    b"u" => fmt.underline = true,
                    b"s" | b"strike" | b"del" => fmt.strike = true,
                    b"a" => fmt.link = attrs.get("href").cloned(),
                    b"br" => {
                        // A line break inside a paragraph: OOXML has <w:br/>, but a paragraph
                        // boundary reads the same and keeps the model simple.
                        if let Some(t) = &mut tbl {
                            if t.in_cell {
                                flush_para(&mut runs, &mut heading, &mut align, -1, &ordered, &mut t.cell);
                                continue;
                            }
                        }
                        flush_para(&mut runs, &mut heading, &mut align, list_depth, &ordered, &mut nodes);
                    }
                    b"img" => {
                        if let Some(src) = attrs.get("src") {
                            let file = src.rsplit('/').next().unwrap_or(src).to_string();
                            if !file.is_empty() && !src.starts_with("data:") {
                                images.push(file.clone());
                                // width/height are CSS pixels; OOXML wants EMUs (914400/inch).
                                let px = |k: &str| attrs.get(k).and_then(|v| v.parse::<f64>().ok());
                                let emu = match (px("width"), px("height")) {
                                    (Some(w), Some(h)) if w > 0.0 && h > 0.0 => Some((
                                        (w / 96.0 * 914400.0).round() as i64,
                                        (h / 96.0 * 914400.0).round() as i64,
                                    )),
                                    _ => None,
                                };
                                runs.push(Run { image: Some(file), image_emu: emu, ..Run::default() });
                            }
                        }
                    }
                    b"hr" => {
                        let cls = attrs.get("class").cloned().unwrap_or_default();
                        if cls.contains("webgen-page-break") || cls.contains("pagebreak") {
                            flush_para(&mut runs, &mut heading, &mut align, list_depth, &ordered, &mut nodes);
                            nodes.push(Node::PageBreak);
                        }
                    }
                    b"h1" | b"h2" | b"h3" | b"h4" | b"h5" | b"h6" => {
                        heading = name[1] - b'0';
                    }
                    b"p" | b"div" => {
                        if let Some(a) = attrs.get("class") {
                            if a.contains("wg-a-center") {
                                align = Some("center".into());
                            } else if a.contains("wg-a-right") {
                                align = Some("right".into());
                            }
                        }
                    }
                    b"ul" => {
                        list_depth += 1;
                        ordered.push(false);
                    }
                    b"ol" => {
                        list_depth += 1;
                        ordered.push(true);
                    }
                    b"table" => {
                        let key = attrs
                            .get("class")
                            .and_then(|c| c.split_whitespace().find(|c| c.starts_with("wg-")))
                            .unwrap_or("")
                            .to_string();
                        tbl = Some(TblState {
                            rows: Vec::new(),
                            row: Vec::new(),
                            cell: Vec::new(),
                            colspan: 1,
                            rowspan: 1,
                            fill: None,
                            header: false,
                            in_cell: false,
                            key,
                        });
                    }
                    b"tr" => {
                        if let Some(t) = &mut tbl {
                            t.row = Vec::new();
                        }
                    }
                    b"td" | b"th" => {
                        if let Some(t) = &mut tbl {
                            t.in_cell = true;
                            t.cell = Vec::new();
                            t.header = name == b"th";
                            t.colspan = attrs
                                .get("colspan")
                                .and_then(|v| v.parse().ok())
                                .unwrap_or(1);
                            t.rowspan = attrs
                                .get("rowspan")
                                .and_then(|v| v.parse().ok())
                                .unwrap_or(1);
                            t.fill = attrs
                                .get("class")
                                .and_then(|c| c.split_whitespace().find(|c| c.starts_with("cell_r")))
                                .and_then(|cls| fill_for(html, &t.key, cls));
                        }
                    }
                    _ => {}
                }
            }
            Event::End(e) => {
                let name = e.name().as_ref().to_ascii_lowercase();
                match name.as_slice() {
                    b"style" | b"script" | b"head" => in_style = false,
                    b"strong" | b"b" => fmt.bold = false,
                    b"em" | b"i" => fmt.italic = false,
                    b"u" => fmt.underline = false,
                    b"s" | b"strike" | b"del" => fmt.strike = false,
                    b"a" => fmt.link = None,
                    b"p" | b"h1" | b"h2" | b"h3" | b"h4" | b"h5" | b"h6" | b"li" | b"div" => {
                        match &mut tbl {
                            Some(t) if t.in_cell => {
                                let mut cell_nodes = std::mem::take(&mut t.cell);
                                flush_para(&mut runs, &mut heading, &mut align, -1, &ordered, &mut cell_nodes);
                                t.cell = cell_nodes;
                            }
                            _ => flush_para(&mut runs, &mut heading, &mut align, list_depth, &ordered, &mut nodes),
                        }
                    }
                    b"ul" | b"ol" => {
                        list_depth -= 1;
                        ordered.pop();
                    }
                    b"td" | b"th" => {
                        if let Some(t) = &mut tbl {
                            let mut cell_nodes = std::mem::take(&mut t.cell);
                            flush_para(&mut runs, &mut heading, &mut align, -1, &ordered, &mut cell_nodes);
                            t.row.push(CellOut {
                                blocks: cell_nodes,
                                colspan: t.colspan.max(1),
                                rowspan: t.rowspan.max(1),
                                fill: t.fill.take(),
                                vmerge_cont: false,
                                header: t.header,
                            });
                            t.in_cell = false;
                        }
                    }
                    b"tr" => {
                        if let Some(t) = &mut tbl {
                            let row = std::mem::take(&mut t.row);
                            t.rows.push(row);
                        }
                    }
                    b"table" => {
                        if let Some(t) = tbl.take() {
                            let mut rows = t.rows;
                            expand_rowspans(&mut rows);
                            nodes.push(Node::Table(TableOut {
                                rows,
                                col_widths_mm: widths.get(&t.key).cloned().unwrap_or_default(),
                                bordered: true,
                            }));
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(t) => {
                if in_style || !in_body {
                    continue;
                }
                // NOT quick-xml's unescape: `&nbsp;` is an HTML entity, not an XML one, so
                // unescape FAILS on it and the whole text node is lost — which silently dropped
                // every hardened pen-fill field. Decode the handful of entities our own documents
                // actually contain, and pass anything else through untouched.
                let text = decode_entities(&String::from_utf8_lossy(t.as_ref()));
                // Non-breaking spaces came from hardened whitespace; they go back out as ordinary
                // spaces inside an xml:space="preserve" run, which is how Word states the same
                // thing natively.
                let text = text.replace('\u{a0}', " ");
                if text.trim().is_empty() && !text.contains(' ') {
                    continue;
                }
                runs.push(Run {
                    text,
                    bold: fmt.bold,
                    italic: fmt.italic,
                    underline: fmt.underline,
                    strike: fmt.strike,
                    link: fmt.link.clone(),
                    image: None,
                    image_emu: None,
                });
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    flush_para(&mut runs, &mut heading, &mut align, list_depth, &ordered, &mut nodes);

    Parsed { nodes, images }
}

/// Decode the entities a WebGen document can contain. Numeric forms are handled generically;
/// the named ones are the short list HTML actually needs here.
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "nbsp" => Some('\u{a0}'),
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            e if e.starts_with("#x") || e.starts_with("#X") => {
                u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32)
            }
            e if e.starts_with('#') => e[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Insert the continuation cells a rowspan implies, so the export can write `vMerge`.
/// Our HTML omits them (the cell above covers the space); OOXML requires them present.
fn expand_rowspans(rows: &mut Vec<Vec<CellOut>>) {
    let mut pending: Vec<(usize, usize, u32)> = Vec::new(); // (row, col, remaining)
    let mut r = 0;
    while r < rows.len() {
        let mut col = 0usize;
        let mut i = 0usize;
        while i <= rows[r].len() {
            // Insert any continuation owed at this column before the next real cell.
            if let Some(p) = pending.iter_mut().find(|(pr, pc, rem)| *pr == r && *pc == col && *rem > 0) {
                let span = 1;
                rows[r].insert(
                    i,
                    CellOut {
                        blocks: Vec::new(),
                        colspan: span,
                        rowspan: 1,
                        fill: None,
                        vmerge_cont: true,
                        header: false,
                    },
                );
                p.2 -= 1;
                if p.2 > 0 {
                    let (_, c, rem) = *p;
                    pending.push((r + 1, c, rem));
                }
                col += 1;
                i += 1;
                continue;
            }
            if i == rows[r].len() {
                break;
            }
            let cell = &rows[r][i];
            if cell.rowspan > 1 {
                pending.push((r + 1, col, cell.rowspan - 1));
            }
            col += cell.colspan as usize;
            i += 1;
        }
        pending.retain(|(pr, _, rem)| *pr > r && *rem > 0);
        r += 1;
    }
}

/// `class -> widths` from the scoped `col.wg-cN { width: Nmm }` rules in the document's style
/// blocks, keyed by the table class they are scoped to.
fn column_widths(html: &str) -> HashMap<String, Vec<f64>> {
    let mut out: HashMap<String, Vec<(usize, f64)>> = HashMap::new();
    for line in html.lines() {
        let line = line.trim();
        // `.wg-t1 col.wg-c2 { width: 40.6mm; }`
        let Some(rest) = line.strip_prefix('.') else { continue };
        let Some((sel, decl)) = rest.split_once('{') else { continue };
        let mut parts = sel.split_whitespace();
        let (Some(table), Some(col)) = (parts.next(), parts.next()) else { continue };
        let Some(idx) = col.strip_prefix("col.wg-c").and_then(|n| n.parse::<usize>().ok()) else {
            continue;
        };
        let Some(mm) = decl
            .split(':')
            .nth(1)
            .and_then(|v| v.trim().trim_end_matches([';', '}', ' ']).strip_suffix("mm"))
            .and_then(|v| v.trim().parse::<f64>().ok())
        else {
            continue;
        };
        out.entry(table.to_string()).or_default().push((idx, mm));
    }
    out.into_iter()
        .map(|(k, mut v)| {
            v.sort_by_key(|(i, _)| *i);
            (k, v.into_iter().map(|(_, w)| w).collect())
        })
        .collect()
}

/// The fill colour a `.cell_rN_cM` rule gives a cell, if any.
fn fill_for(html: &str, table_class: &str, cell_class: &str) -> Option<String> {
    let needle = format!(".{table_class} .{cell_class}");
    for line in html.lines() {
        let line = line.trim();
        if line.starts_with(&needle) {
            if let Some((_, decl)) = line.split_once('{') {
                if let Some(v) = decl.split(':').nth(1) {
                    let v = v.trim().trim_end_matches([';', '}', ' ']).trim();
                    if v.starts_with('#') && v.len() == 7 {
                        return Some(v.to_string());
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraphs_headings_and_emphasis_read_back() {
        let p = parse("<body><h2>Title</h2><p>plain <strong>bold</strong> <em>it</em></p></body>");
        assert_eq!(p.nodes.len(), 2);
        match &p.nodes[0] {
            Node::Para { level, runs, .. } => {
                assert_eq!(*level, 2);
                assert_eq!(runs[0].text, "Title");
            }
            other => panic!("{other:?}"),
        }
        match &p.nodes[1] {
            Node::Para { runs, .. } => {
                assert!(runs.iter().any(|r| r.bold && r.text == "bold"), "{runs:?}");
                assert!(runs.iter().any(|r| r.italic && r.text == "it"), "{runs:?}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn lists_carry_their_depth_and_kind() {
        let p = parse("<body><ul><li>a</li><ol><li>b</li></ol></ul></body>");
        let items: Vec<&Node> = p.nodes.iter().filter(|n| matches!(n, Node::Item { .. })).collect();
        assert_eq!(items.len(), 2);
        match items[0] {
            Node::Item { ordered, depth, .. } => {
                assert!(!ordered);
                assert_eq!(*depth, 0);
            }
            other => panic!("{other:?}"),
        }
        match items[1] {
            Node::Item { ordered, depth, .. } => {
                assert!(ordered);
                assert_eq!(*depth, 1);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_table_keeps_its_widths_spans_and_fills() {
        let html = r#"<body>
<style>
table.wg-t1 { table-layout: fixed; width: 150.0mm; }
.wg-t1 col.wg-c1 { width: 50.0mm; }
.wg-t1 col.wg-c2 { width: 100.0mm; }
.wg-t1 .cell_r1_c1 { background: #d9d9d9; }
</style>
<table class="wg-t1"><colgroup><col class="wg-c1"><col class="wg-c2"></colgroup>
<tbody>
<tr><td class="cell_r1_c1" rowspan="2">tall</td><td>b</td></tr>
<tr><td>c</td></tr>
</tbody></table></body>"#;
        let p = parse(html);
        let Node::Table(t) = p.nodes.iter().find(|n| matches!(n, Node::Table(_))).unwrap() else {
            unreachable!()
        };
        assert_eq!(t.col_widths_mm, vec![50.0, 100.0]);
        assert_eq!(t.rows[0][0].rowspan, 2);
        assert_eq!(t.rows[0][0].fill.as_deref(), Some("#d9d9d9"));
        // The continuation cell OOXML needs was inserted for the second row.
        assert!(t.rows[1].iter().any(|c| c.vmerge_cont), "{:?}", t.rows[1]);
    }

    #[test]
    fn page_breaks_and_hardened_spaces_survive() {
        let p = parse("<body><p>a&nbsp;&nbsp; b</p><hr class=\"webgen-page-break\"><p>next</p></body>");
        assert!(p.nodes.iter().any(|n| matches!(n, Node::PageBreak)));
        match &p.nodes[0] {
            Node::Para { runs, .. } => assert!(runs[0].text.contains("a   b"), "{runs:?}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn images_are_collected_by_file_name() {
        let p = parse(r#"<body><p><img src="doc_files/logo.png"></p></body>"#);
        assert_eq!(p.images, vec!["logo.png"]);
        match &p.nodes[0] {
            Node::Para { runs, .. } => assert_eq!(runs[0].image.as_deref(), Some("logo.png")),
            other => panic!("{other:?}"),
        }
    }
}
