//! docx -> HTML.
//!
//! A .docx is a DEFLATED zip: `word/document.xml` is the text, `word/styles.xml` carries the
//! style definitions (including the numbering a template hangs on styles like "Bullet-main"),
//! `word/numbering.xml` says which lists are bullets and which are numbered, the rels file maps
//! r:id references to hyperlink targets and media files. The Uni corpus this was built against is
//! table-heavy assessment forms: tables with gridSpan/vMerge merges dominate, headings and
//! style-carried bullets follow, images are rare. That ordering is why tables get the most care.

use crate::Converted;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::collections::HashMap;
use std::io::Read;

pub fn docx_to_html(docx: &[u8], asset_dir: &str) -> Result<Converted, String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(docx))
        .map_err(|e| format!("not a readable zip: {e}"))?;

    let document = read_entry(&mut zip, "word/document.xml")?
        .ok_or("no word/document.xml — this zip is not a docx")?;
    let styles = read_entry(&mut zip, "word/styles.xml")?.unwrap_or_default();
    let numbering = read_entry(&mut zip, "word/numbering.xml")?.unwrap_or_default();
    let rels = read_entry(&mut zip, "word/_rels/document.xml.rels")?.unwrap_or_default();

    // Media files are pulled lazily by rel target when a drawing references them.
    let mut media: HashMap<String, Vec<u8>> = HashMap::new();
    for i in 0..zip.len() {
        let name = zip.by_index(i).map_err(|e| e.to_string())?.name().to_string();
        if name.starts_with("word/media/") {
            if let Some(bytes) = read_entry(&mut zip, &name)? {
                media.insert(name.trim_start_matches("word/").to_string(), bytes);
            }
        }
    }

    let mut p = Parser {
        styles: parse_styles(&styles),
        numbering: parse_numbering(&numbering),
        rels: parse_rels(&rels),
        media,
        asset_dir: asset_dir.to_string(),
        assets: Vec::new(),
        notes: Vec::new(),
        in_field_instruction: false,
    };

    let body_html = p.parse_document(&document)?;
    Ok(Converted {
        body_html,
        assets: p.assets,
        notes: p.notes,
    })
}

fn read_entry(
    zip: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
    name: &str,
) -> Result<Option<Vec<u8>>, String> {
    match zip.by_name(name) {
        Ok(mut f) => {
            let mut out = Vec::new();
            f.read_to_end(&mut out).map_err(|e| format!("{name}: {e}"))?;
            Ok(Some(out))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(e) => Err(format!("{name}: {e}")),
    }
}

// ---- attribute helpers ------------------------------------------------------------------------

/// Attribute by local name, prefix-blind: `w:val`, `r:id` and plain `val` all answer to their
/// suffix. docx never uses two prefixes for the same local name on one element.
fn attr(e: &BytesStart, local: &str) -> Option<String> {
    for a in e.attributes().flatten() {
        let key = a.key.as_ref();
        let matches = key == local.as_bytes()
            || (key.len() > local.len()
                && key.ends_with(local.as_bytes())
                && key[key.len() - local.len() - 1] == b':');
        if matches {
            return a.unescape_value().ok().map(|v| v.to_string());
        }
    }
    None
}

fn local_name(qname: &[u8]) -> &[u8] {
    match qname.iter().rposition(|&b| b == b':') {
        Some(i) => &qname[i + 1..],
        None => qname,
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attr(s: &str) -> String {
    escape(s).replace('"', "&quot;")
}

// ---- styles.xml -------------------------------------------------------------------------------

#[derive(Default, Clone)]
struct Style {
    heading: Option<u8>,               // 1..=6
    num: Option<(String, u32)>,        // (numId, ilvl) hung on the style
    based_on: Option<String>,
}

fn parse_styles(xml: &[u8]) -> HashMap<String, Style> {
    let mut out: HashMap<String, Style> = HashMap::new();
    if xml.is_empty() {
        return out;
    }
    let mut r = Reader::from_reader(xml);
    r.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut cur_id: Option<String> = None;
    let mut cur = Style::default();
    let mut num_id: Option<String> = None;
    let mut ilvl: u32 = 0;
    loop {
        match r.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => match local_name(e.name().as_ref()) {
                b"style" => {
                    cur_id = attr(&e, "styleId");
                    cur = Style::default();
                    num_id = None;
                    ilvl = 0;
                }
                b"basedOn" => cur.based_on = attr(&e, "val"),
                b"outlineLvl" => {
                    if let Some(v) = attr(&e, "val").and_then(|v| v.parse::<u8>().ok()) {
                        if v < 6 {
                            cur.heading = Some(v + 1);
                        }
                    }
                }
                b"numId" => num_id = attr(&e, "val"),
                b"ilvl" => ilvl = attr(&e, "val").and_then(|v| v.parse().ok()).unwrap_or(0),
                _ => {}
            },
            Ok(Event::End(e)) if local_name(e.name().as_ref()) == b"style" => {
                if let Some(id) = cur_id.take() {
                    // "Heading1".."Heading6" beats outlineLvl when both are present — the id is
                    // what the documents in the wild actually key on.
                    if let Some(level) = id
                        .strip_prefix("Heading")
                        .and_then(|n| n.parse::<u8>().ok())
                        .filter(|n| (1..=6).contains(n))
                    {
                        cur.heading = Some(level);
                    }
                    if let Some(n) = num_id.take() {
                        if n != "0" {
                            cur.num = Some((n, ilvl));
                        }
                    }
                    out.insert(id, cur.clone());
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

/// Follow the basedOn chain until a heading/num is found. Bounded — templates in the wild have
/// contained basedOn cycles, and an infinite loop over a hostile file is not an option.
fn resolve<'a, T, F: Fn(&'a Style) -> Option<T>>(
    styles: &'a HashMap<String, Style>,
    id: &str,
    pick: F,
) -> Option<T> {
    let mut cur = styles.get(id);
    for _ in 0..8 {
        let s = cur?;
        if let Some(v) = pick(s) {
            return Some(v);
        }
        cur = s.based_on.as_deref().and_then(|b| styles.get(b));
    }
    None
}

// ---- numbering.xml ----------------------------------------------------------------------------

/// numId -> ilvl -> ordered?
fn parse_numbering(xml: &[u8]) -> HashMap<String, HashMap<u32, bool>> {
    let mut abstract_fmt: HashMap<String, HashMap<u32, bool>> = HashMap::new();
    let mut num_to_abstract: HashMap<String, String> = HashMap::new();
    if xml.is_empty() {
        return HashMap::new();
    }
    let mut r = Reader::from_reader(xml);
    r.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut cur_abstract: Option<String> = None;
    let mut cur_num: Option<String> = None;
    let mut cur_ilvl: u32 = 0;
    loop {
        match r.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => match local_name(e.name().as_ref()) {
                b"abstractNum" => cur_abstract = attr(&e, "abstractNumId"),
                b"lvl" => cur_ilvl = attr(&e, "ilvl").and_then(|v| v.parse().ok()).unwrap_or(0),
                b"numFmt" => {
                    if let (Some(a), Some(v)) = (cur_abstract.clone(), attr(&e, "val")) {
                        let ordered = v != "bullet" && v != "none";
                        abstract_fmt.entry(a).or_default().insert(cur_ilvl, ordered);
                    }
                }
                b"num" => cur_num = attr(&e, "numId"),
                b"abstractNumId" => {
                    if let (Some(n), Some(a)) = (cur_num.clone(), attr(&e, "val")) {
                        num_to_abstract.insert(n, a);
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    num_to_abstract
        .into_iter()
        .filter_map(|(num, abs)| abstract_fmt.get(&abs).cloned().map(|m| (num, m)))
        .collect()
}

// ---- rels -------------------------------------------------------------------------------------

struct Rel {
    target: String,
    external: bool,
}

fn parse_rels(xml: &[u8]) -> HashMap<String, Rel> {
    let mut out = HashMap::new();
    if xml.is_empty() {
        return out;
    }
    let mut r = Reader::from_reader(xml);
    r.config_mut().trim_text(false);
    let mut buf = Vec::new();
    loop {
        match r.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"Relationship" {
                    if let (Some(id), Some(target)) = (attr(&e, "Id"), attr(&e, "Target")) {
                        let external =
                            attr(&e, "TargetMode").as_deref() == Some("External");
                        out.insert(id, Rel { target, external });
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

// ---- the document walk ------------------------------------------------------------------------

struct Parser {
    styles: HashMap<String, Style>,
    numbering: HashMap<String, HashMap<u32, bool>>,
    rels: HashMap<String, Rel>,
    media: HashMap<String, Vec<u8>>,
    asset_dir: String,
    assets: Vec<(String, Vec<u8>)>,
    notes: Vec<String>,
    /// Between a field's "begin" and its "separate", runs carry the field INSTRUCTION
    /// (`TOC \o "1-3"`, `HYPERLINK "…"`), not content. Those must not leak into the text.
    in_field_instruction: bool,
}

/// One block-level thing inside a body or a table cell.
enum Block {
    Para {
        tag: &'static str,          // "p" or "h1".."h6"
        inner: String,
        num: Option<(bool, u32)>,   // (ordered, ilvl) when the paragraph is a list item
    },
    Html(String),                    // a finished table
}

impl Parser {
    fn parse_document(&mut self, xml: &[u8]) -> Result<String, String> {
        let mut r = Reader::from_reader(xml);
        r.config_mut().trim_text(false);
        let mut buf = Vec::new();
        let blocks = self.parse_blocks(&mut r, &mut buf, b"body")?;
        Ok(render_blocks(blocks))
    }

    /// Consume block-level content until `</w:{end}>`, returning the blocks in order. Recurses
    /// into tables; the list-nesting decisions happen later, in `render_blocks`.
    fn parse_blocks(
        &mut self,
        r: &mut Reader<&[u8]>,
        buf: &mut Vec<u8>,
        end: &[u8],
    ) -> Result<Vec<Block>, String> {
        let mut out = Vec::new();
        loop {
            let ev = r.read_event_into(buf).map_err(|e| e.to_string())?;
            match ev {
                Event::Start(e) => match local_name(e.name().as_ref()) {
                    b"p" => {
                        let b = self.parse_paragraph(r)?;
                        out.push(b);
                    }
                    b"tbl" => {
                        let html = self.parse_table(r)?;
                        out.push(Block::Html(html));
                    }
                    _ => {}
                },
                Event::End(e) if local_name(e.name().as_ref()) == end => break,
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        Ok(out)
    }

    /// Everything from after `<w:p>` to `</w:p>`.
    fn parse_paragraph(&mut self, r: &mut Reader<&[u8]>) -> Result<Block, String> {
        let mut buf = Vec::new();
        let mut inner = String::new();
        let mut style_id: Option<String> = None;
        let mut inline_num: Option<(Option<String>, u32)> = None; // (numId, ilvl) as written
        let mut in_ppr = false;
        // Run formatting is collected while inside <w:rPr> and applied when the run's text lands.
        let mut fmt = RunFmt::default();
        let mut in_rpr = false;
        let mut link: Option<String> = None; // open <a href> to close at </w:hyperlink>
        let mut in_del = false;

        loop {
            let ev = r.read_event_into(&mut buf).map_err(|e| e.to_string())?;
            match ev {
                Event::Start(ref e) | Event::Empty(ref e) => {
                    let name = local_name(e.name().as_ref()).to_vec();
                    let empty = matches!(ev, Event::Empty(_));
                    match name.as_slice() {
                        b"pPr" if !empty => in_ppr = true,
                        b"pStyle" if in_ppr => style_id = attr(e, "val"),
                        b"numPr" if in_ppr => inline_num = Some((None, 0)),
                        b"ilvl" if in_ppr => {
                            if let Some(n) = inline_num.as_mut() {
                                n.1 = attr(e, "val").and_then(|v| v.parse().ok()).unwrap_or(0);
                            }
                        }
                        b"numId" if in_ppr => {
                            if let Some(n) = inline_num.as_mut() {
                                n.0 = attr(e, "val");
                            }
                        }
                        b"rPr" if !empty => {
                            in_rpr = true;
                            fmt = RunFmt::default();
                        }
                        b"b" if in_rpr => fmt.bold = attr(e, "val").as_deref() != Some("0")
                            && attr(e, "val").as_deref() != Some("false"),
                        b"i" if in_rpr => fmt.italic = attr(e, "val").as_deref() != Some("0")
                            && attr(e, "val").as_deref() != Some("false"),
                        b"u" if in_rpr => fmt.underline = attr(e, "val").as_deref() != Some("none"),
                        b"strike" if in_rpr => fmt.strike = attr(e, "val").as_deref() != Some("0")
                            && attr(e, "val").as_deref() != Some("false"),
                        b"vertAlign" if in_rpr => match attr(e, "val").as_deref() {
                            Some("superscript") => fmt.vert = Vert::Sup,
                            Some("subscript") => fmt.vert = Vert::Sub,
                            _ => fmt.vert = Vert::None,
                        },
                        b"br" => inner.push_str("<br>"),
                        b"tab" if !in_ppr => inner.push('\u{2003}'), // em space stands in for the tab stop
                        b"fldChar" => match attr(e, "fldCharType").as_deref() {
                            Some("begin") => self.in_field_instruction = true,
                            _ => self.in_field_instruction = false,
                        },
                        b"hyperlink" if !empty => {
                            let href = attr(e, "id")
                                .and_then(|id| self.rels.get(&id))
                                .filter(|rel| rel.external)
                                .map(|rel| rel.target.clone());
                            if let Some(h) = href {
                                inner.push_str(&format!("<a href=\"{}\">", escape_attr(&h)));
                                link = Some(h);
                            }
                        }
                        b"del" if !empty => in_del = true,
                        b"drawing" if !empty => {
                            let img = self.parse_drawing(r)?;
                            if !in_del {
                                inner.push_str(&img);
                            }
                        }
                        _ => {}
                    }
                }
                Event::Text(t) => {
                    if !self.in_field_instruction && !in_del {
                        // instrText is skipped wholesale via the flag; ordinary w:t lands here.
                        let text = t.unescape().map_err(|e| e.to_string())?;
                        inner.push_str(&fmt.wrap(&escape(&text)));
                    }
                }
                Event::End(e) => match local_name(e.name().as_ref()) {
                    b"p" => break,
                    b"pPr" => in_ppr = false,
                    b"rPr" => in_rpr = false,
                    b"r" => fmt = RunFmt::default(),
                    b"hyperlink" => {
                        if link.take().is_some() {
                            inner.push_str("</a>");
                        }
                    }
                    b"del" => in_del = false,
                    _ => {}
                },
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }

        // Effective heading and numbering: inline first, then the style chain — templates hang
        // bullets on styles ("Bullet-main"), and missing that is missing half the lists.
        // The "Heading1".."Heading6" id convention holds even when styles.xml is absent or
        // doesn't restate it, so it is checked before the style table.
        let heading = style_id
            .as_deref()
            .and_then(|id| {
                id.strip_prefix("Heading")
                    .and_then(|n| n.parse::<u8>().ok())
                    .filter(|n| (1..=6).contains(n))
            })
            .or_else(|| {
                style_id
                    .as_deref()
                    .and_then(|id| resolve(&self.styles, id, |s| s.heading))
            });
        let num_pair: Option<(String, u32)> = match inline_num {
            Some((Some(id), ilvl)) if id != "0" => Some((id, ilvl)),
            Some((Some(_zero), _)) => None, // numId 0 explicitly REMOVES a style's numbering
            _ => style_id
                .as_deref()
                .and_then(|id| resolve(&self.styles, id, |s| s.num.clone())),
        };
        let num = num_pair.map(|(id, ilvl)| {
            let ordered = self
                .numbering
                .get(&id)
                .and_then(|m| m.get(&ilvl))
                .copied()
                .unwrap_or(false);
            (ordered, ilvl)
        });

        let tag: &'static str = match heading {
            Some(1) => "h1",
            Some(2) => "h2",
            Some(3) => "h3",
            Some(4) => "h4",
            Some(5) => "h5",
            Some(6) => "h6",
            _ => "p",
        };
        Ok(Block::Para { tag, inner, num })
    }

    /// `<w:drawing>`: find the blip's r:embed, extract the media file, reference it.
    fn parse_drawing(&mut self, r: &mut Reader<&[u8]>) -> Result<String, String> {
        let mut buf = Vec::new();
        let mut html = String::new();
        loop {
            match r.read_event_into(&mut buf).map_err(|e| e.to_string())? {
                Event::Start(e) | Event::Empty(e) => {
                    if local_name(e.name().as_ref()) == b"blip" {
                        if let Some(rel) = attr(&e, "embed").and_then(|id| self.rels.get(&id)) {
                            let target = rel.target.trim_start_matches("./").to_string();
                            if let Some(bytes) = self.media.get(&target).cloned() {
                                let base = target.rsplit('/').next().unwrap_or("image").to_string();
                                let name = self.unique_asset_name(&base);
                                html = format!(
                                    "<img src=\"{}/{}\">",
                                    escape_attr(&self.asset_dir),
                                    escape_attr(&name)
                                );
                                self.assets.push((name, bytes));
                            } else {
                                self.notes.push(format!("a picture ({target}) could not be extracted"));
                            }
                        }
                    }
                }
                Event::End(e) if local_name(e.name().as_ref()) == b"drawing" => break,
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        Ok(html)
    }

    fn unique_asset_name(&self, wanted: &str) -> String {
        let taken = |n: &str| self.assets.iter().any(|(existing, _)| existing == n);
        if !taken(wanted) {
            return wanted.to_string();
        }
        let (stem, ext) = match wanted.rsplit_once('.') {
            Some((s, e)) => (s.to_string(), format!(".{e}")),
            None => (wanted.to_string(), String::new()),
        };
        for i in 1.. {
            let cand = format!("{stem}-{i}{ext}");
            if !taken(&cand) {
                return cand;
            }
        }
        unreachable!()
    }

    /// `<w:tbl>` … `</w:tbl>` — buffered, because vMerge rowspans cannot be emitted streaming:
    /// the cell that grows is ABOVE the cells that vanish.
    fn parse_table(&mut self, r: &mut Reader<&[u8]>) -> Result<String, String> {
        struct Cell {
            html: String,
            colspan: u32,
            vmerge: Option<bool>, // Some(true)=restart, Some(false)=continue
            rowspan: u32,
        }
        let mut rows: Vec<Vec<Cell>> = Vec::new();
        let mut buf = Vec::new();
        loop {
            let ev = r.read_event_into(&mut buf).map_err(|e| e.to_string())?;
            match ev {
                Event::Start(ref e) => match local_name(e.name().as_ref()) {
                    b"tr" => rows.push(Vec::new()),
                    b"tc" => {
                        let (html, colspan, vmerge) = self.parse_cell(r)?;
                        rows.last_mut()
                            .ok_or("a table cell outside any row")?
                            .push(Cell { html, colspan, vmerge, rowspan: 1 });
                    }
                    b"tbl" => {
                        // A nested table before any cell would be malformed; inside cells it is
                        // handled by parse_cell. Reaching here means stray markup — skip it.
                        let _ = self.parse_table(r)?;
                    }
                    _ => {}
                },
                Event::End(e) if local_name(e.name().as_ref()) == b"tbl" => break,
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }

        // Resolve vMerge into rowspans. Column positions are tracked with colspans so a merge in
        // a ragged form lines up with the right upstairs cell.
        let mut open: HashMap<u32, (usize, usize)> = HashMap::new(); // col -> (row, cell)
        for ri in 0..rows.len() {
            let mut col = 0u32;
            for ci in 0..rows[ri].len() {
                let (span, merge) = (rows[ri][ci].colspan, rows[ri][ci].vmerge);
                match merge {
                    Some(true) => {
                        open.insert(col, (ri, ci));
                    }
                    Some(false) => {
                        if let Some(&(orow, ocell)) = open.get(&col) {
                            rows[orow][ocell].rowspan += 1;
                        }
                    }
                    None => {
                        open.remove(&col);
                    }
                }
                col += span;
            }
        }

        let mut html = String::from("<table>");
        for (ri, row) in rows.iter().enumerate() {
            html.push_str("<tr>");
            for cell in row {
                if cell.vmerge == Some(false) {
                    continue; // swallowed by the rowspan above it
                }
                // First row as headers: the corpus is assessment forms, where it holds. A wrong
                // th renders bold-centred — a visible nudge, not data loss.
                let tag = if ri == 0 { "th" } else { "td" };
                let mut attrs = String::new();
                if cell.colspan > 1 {
                    attrs.push_str(&format!(" colspan=\"{}\"", cell.colspan));
                }
                if cell.rowspan > 1 {
                    attrs.push_str(&format!(" rowspan=\"{}\"", cell.rowspan));
                }
                html.push_str(&format!("<{tag}{attrs}>{}</{tag}>", cell.html));
            }
            html.push_str("</tr>");
        }
        html.push_str("</table>");
        Ok(html)
    }

    /// One `<w:tc>` … `</w:tc>`: harvest the props (gridSpan/vMerge live inside `<w:tcPr>`,
    /// always before any content) and parse the block content in the same loop — no peeking,
    /// and a cell WITHOUT a tcPr loses nothing.
    fn parse_cell(&mut self, r: &mut Reader<&[u8]>) -> Result<(String, u32, Option<bool>), String> {
        let mut buf = Vec::new();
        let mut colspan = 1u32;
        let mut vmerge: Option<bool> = None;
        let mut blocks: Vec<Block> = Vec::new();
        loop {
            let ev = r.read_event_into(&mut buf).map_err(|e| e.to_string())?;
            match ev {
                Event::Start(ref e) => match local_name(e.name().as_ref()) {
                    b"p" => {
                        let b = self.parse_paragraph(r)?;
                        blocks.push(b);
                    }
                    b"tbl" => {
                        let html = self.parse_table(r)?;
                        blocks.push(Block::Html(html));
                    }
                    // Some producers write the props as non-empty elements.
                    b"gridSpan" => {
                        colspan = attr(e, "val").and_then(|v| v.parse().ok()).unwrap_or(1)
                    }
                    b"vMerge" => vmerge = Some(attr(e, "val").as_deref() == Some("restart")),
                    _ => {}
                },
                Event::Empty(ref e) => match local_name(e.name().as_ref()) {
                    b"gridSpan" => {
                        colspan = attr(e, "val").and_then(|v| v.parse().ok()).unwrap_or(1)
                    }
                    b"vMerge" => vmerge = Some(attr(e, "val").as_deref() == Some("restart")),
                    _ => {}
                },
                Event::End(e) if local_name(e.name().as_ref()) == b"tc" => break,
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        Ok((render_blocks(blocks), colspan, vmerge))
    }
}

// ---- run formatting ---------------------------------------------------------------------------

#[derive(Default, Clone)]
struct RunFmt {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    vert: Vert,
}

#[derive(Default, Clone, PartialEq)]
enum Vert {
    #[default]
    None,
    Sup,
    Sub,
}

impl RunFmt {
    fn wrap(&self, text: &str) -> String {
        if text.is_empty() {
            return String::new();
        }
        let mut out = text.to_string();
        if self.vert == Vert::Sup {
            out = format!("<sup>{out}</sup>");
        }
        if self.vert == Vert::Sub {
            out = format!("<sub>{out}</sub>");
        }
        if self.strike {
            out = format!("<s>{out}</s>");
        }
        if self.underline {
            out = format!("<u>{out}</u>");
        }
        if self.italic {
            out = format!("<em>{out}</em>");
        }
        if self.bold {
            out = format!("<strong>{out}</strong>");
        }
        out
    }
}

// ---- list nesting -----------------------------------------------------------------------------

/// Turn the flat block list into HTML, folding consecutive numbered paragraphs into nested
/// `<ul>`/`<ol>` by their ilvl. Word writes lists as flat runs of paragraphs; the nesting exists
/// only in the ilvl numbers.
fn render_blocks(blocks: Vec<Block>) -> String {
    let mut out = String::new();
    let mut stack: Vec<bool> = Vec::new(); // ordered? per open level

    let close_to = |out: &mut String, stack: &mut Vec<bool>, depth: usize| {
        while stack.len() > depth {
            out.push_str(if stack.pop().unwrap() { "</ol>" } else { "</ul>" });
        }
    };

    for b in blocks {
        match b {
            Block::Para { tag, inner, num } => match num {
                Some((ordered, ilvl)) => {
                    let want = ilvl as usize + 1;
                    // Deeper: open levels down to the item's. Shallower: close back up.
                    close_to(&mut out, &mut stack, want);
                    while stack.len() < want {
                        out.push_str(if ordered { "<ol>" } else { "<ul>" });
                        stack.push(ordered);
                    }
                    // Same depth but the list KIND changed: close and reopen.
                    if *stack.last().unwrap() != ordered {
                        out.push_str(if stack.pop().unwrap() { "</ol>" } else { "</ul>" });
                        out.push_str(if ordered { "<ol>" } else { "<ul>" });
                        stack.push(ordered);
                    }
                    out.push_str(&format!("<li>{inner}</li>"));
                }
                None => {
                    close_to(&mut out, &mut stack, 0);
                    if inner.is_empty() {
                        out.push_str("<p><br></p>");
                    } else {
                        out.push_str(&format!("<{tag}>{inner}</{tag}>"));
                    }
                }
            },
            Block::Html(h) => {
                close_to(&mut out, &mut stack, 0);
                out.push_str(&h);
            }
        }
    }
    close_to(&mut out, &mut stack, 0);
    out
}

// ---- tests ------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Build an in-memory docx with the given entries.
    fn docx(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut cur = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut cur);
            let opt = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for (name, body) in entries {
                z.start_file(*name, opt).unwrap();
                z.write_all(body.as_bytes()).unwrap();
            }
            z.finish().unwrap();
        }
        cur.into_inner()
    }

    fn doc(body: &str) -> String {
        format!(
            r#"<?xml version="1.0"?><w:document xmlns:w="w" xmlns:r="r"><w:body>{body}</w:body></w:document>"#
        )
    }

    #[test]
    fn paragraphs_headings_and_formatting() {
        let d = docx(&[(
            "word/document.xml",
            &doc(concat!(
                r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Title</w:t></w:r></w:p>"#,
                r#"<w:p><w:r><w:rPr><w:b/></w:rPr><w:t>bold</w:t></w:r>"#,
                r#"<w:r><w:t xml:space="preserve"> and </w:t></w:r>"#,
                r#"<w:r><w:rPr><w:i/></w:rPr><w:t>italic</w:t></w:r></w:p>"#,
            )),
        )]);
        let c = docx_to_html(&d, "pics").unwrap();
        assert_eq!(
            c.body_html,
            "<h1>Title</h1><p><strong>bold</strong> and <em>italic</em></p>"
        );
    }

    #[test]
    fn lists_nest_by_ilvl_and_kind_comes_from_numbering() {
        let numbering = concat!(
            r#"<?xml version="1.0"?><w:numbering xmlns:w="w">"#,
            r#"<w:abstractNum w:abstractNumId="0">"#,
            r#"<w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/></w:lvl>"#,
            r#"<w:lvl w:ilvl="1"><w:numFmt w:val="decimal"/></w:lvl>"#,
            r#"</w:abstractNum>"#,
            r#"<w:num w:numId="5"><w:abstractNumId w:val="0"/></w:num>"#,
            r#"</w:numbering>"#
        );
        let li = |ilvl: u32, text: &str| {
            format!(
                r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="{ilvl}"/><w:numId w:val="5"/></w:numPr></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#
            )
        };
        let d = docx(&[
            ("word/document.xml", &doc(&format!("{}{}{}", li(0, "a"), li(1, "b"), li(0, "c")))),
            ("word/numbering.xml", numbering),
        ]);
        let c = docx_to_html(&d, "pics").unwrap();
        assert_eq!(
            c.body_html,
            "<ul><li>a</li><ol><li>b</li></ol><li>c</li></ul>"
        );
    }

    #[test]
    fn style_carried_numbering_makes_list_items() {
        // The template trick: no numPr on the paragraph, the STYLE carries it.
        let styles = concat!(
            r#"<?xml version="1.0"?><w:styles xmlns:w="w">"#,
            r#"<w:style w:styleId="BulletMain"><w:pPr><w:numPr>"#,
            r#"<w:ilvl w:val="0"/><w:numId w:val="7"/></w:numPr></w:pPr></w:style>"#,
            r#"</w:styles>"#
        );
        let numbering = concat!(
            r#"<?xml version="1.0"?><w:numbering xmlns:w="w">"#,
            r#"<w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/></w:lvl></w:abstractNum>"#,
            r#"<w:num w:numId="7"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#
        );
        let d = docx(&[
            (
                "word/document.xml",
                &doc(r#"<w:p><w:pPr><w:pStyle w:val="BulletMain"/></w:pPr><w:r><w:t>styled</w:t></w:r></w:p>"#),
            ),
            ("word/styles.xml", styles),
            ("word/numbering.xml", numbering),
        ]);
        let c = docx_to_html(&d, "pics").unwrap();
        assert_eq!(c.body_html, "<ul><li>styled</li></ul>");
    }

    #[test]
    fn tables_merge_cells_both_ways() {
        let cell = |props: &str, text: &str| {
            format!(r#"<w:tc><w:tcPr>{props}</w:tcPr><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>"#)
        };
        let body = format!(
            "<w:tbl><w:tr>{}{}</w:tr><w:tr>{}{}{}</w:tr><w:tr>{}{}{}</w:tr></w:tbl>",
            cell(r#"<w:gridSpan w:val="2"/>"#, "head-wide"),
            cell("", "head-b"),
            cell(r#"<w:vMerge w:val="restart"/>"#, "tall"),
            cell("", "r1c2"),
            cell("", "r1c3"),
            cell(r#"<w:vMerge/>"#, ""),
            cell("", "r2c2"),
            cell("", "r2c3"),
        );
        let d = docx(&[("word/document.xml", &doc(&body))]);
        let c = docx_to_html(&d, "pics").unwrap();
        assert_eq!(
            c.body_html,
            concat!(
                "<table>",
                "<tr><th colspan=\"2\"><p>head-wide</p></th><th><p>head-b</p></th></tr>",
                "<tr><td rowspan=\"2\"><p>tall</p></td><td><p>r1c2</p></td><td><p>r1c3</p></td></tr>",
                "<tr><td><p>r2c2</p></td><td><p>r2c3</p></td></tr>",
                "</table>"
            )
        );
    }

    #[test]
    fn hyperlinks_resolve_through_rels_and_field_instructions_stay_out() {
        let rels = concat!(
            r#"<?xml version="1.0"?><Relationships xmlns="rel">"#,
            r#"<Relationship Id="rId9" Target="https://example.com/x" TargetMode="External"/>"#,
            r#"</Relationships>"#
        );
        let body = concat!(
            r#"<w:p><w:hyperlink r:id="rId9"><w:r><w:t>a link</w:t></w:r></w:hyperlink></w:p>"#,
            // A TOC field: the instruction text must not appear in the output.
            r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r>"#,
            r#"<w:r><w:instrText>TOC \o "1-3"</w:instrText></w:r>"#,
            r#"<w:r><w:fldChar w:fldCharType="separate"/></w:r>"#,
            r#"<w:r><w:t>Visible entry</w:t></w:r>"#,
            r#"<w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#
        );
        let d = docx(&[
            ("word/document.xml", &doc(body)),
            ("word/_rels/document.xml.rels", rels),
        ]);
        let c = docx_to_html(&d, "pics").unwrap();
        assert_eq!(
            c.body_html,
            "<p><a href=\"https://example.com/x\">a link</a></p><p>Visible entry</p>"
        );
    }

    #[test]
    fn text_is_escaped_and_tracked_deletions_are_dropped() {
        let body = concat!(
            r#"<w:p><w:r><w:t>a &lt;b&gt; &amp; c</w:t></w:r>"#,
            r#"<w:del><w:r><w:t>gone</w:t></w:r></w:del>"#,
            r#"<w:ins><w:r><w:t> kept</w:t></w:r></w:ins></w:p>"#
        );
        let d = docx(&[("word/document.xml", &doc(body))]);
        let c = docx_to_html(&d, "pics").unwrap();
        assert_eq!(c.body_html, "<p>a &lt;b&gt; &amp; c kept</p>");
    }

    #[test]
    fn a_zip_that_is_not_a_docx_says_so() {
        let d = docx(&[("hello.txt", "nope")]);
        let err = docx_to_html(&d, "pics").unwrap_err();
        assert!(err.contains("not a docx"), "{err}");
    }
}
