//! HTML -> docx. The import mapping, run backwards.
//!
//! Piers's acceptance test (2026-08-06) is a round trip with a human at the far end: the
//! lecturer's own `.docx` opened in WebGen Word, edited, saved back as `.docx`, and emailed to
//! him — the person who wrote the original — for comment. So the bar here is not "a file that
//! opens" but "a file that reads as the same document to its author".
//!
//! ## What that means in practice
//!
//! - **Structure over decoration.** Headings, paragraphs, lists, tables with their merges and
//!   their column widths, bold/italic/underline, links, page geometry. These carry the meaning.
//! - **A minimal, VALID package.** `[Content_Types].xml`, `_rels/.rels`, `word/document.xml`,
//!   `word/styles.xml`, `word/_rels/document.xml.rels`, and `word/media/*` when there are
//!   pictures. Word and LibreOffice both reject a package that misses any of the first four.
//! - **Twips everywhere.** OOXML measures in twentieths of a point: 1440 per inch, 567 per cm.
//!   Our HTML measures in millimetres, so every width crosses that boundary once, here.
//!
//! ## What it deliberately does not do
//!
//! Round-tripping CSS into Word styles is a rabbit hole with no bottom — a document's own
//! stylesheet can say things OOXML has no vocabulary for. The export writes the STRUCTURE
//! faithfully and lets Word's defaults dress it, rather than inventing a style hierarchy that
//! would fight the recipient's template. Colour, fonts and per-cell shading are the exceptions:
//! they survive because their absence is what makes a document look broken rather than plain.

use std::collections::HashMap;
use std::io::Write;

/// One block of the document to write, as the HTML reader understood it.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    /// Heading level 1-6, or 0 for a plain paragraph.
    Para { level: u8, runs: Vec<Run>, align: Option<String> },
    /// A list item at `depth` (0-based), ordered or not.
    Item { ordered: bool, depth: u8, runs: Vec<Run> },
    Table(TableOut),
    /// A page break — `<hr class="webgen-page-break">`.
    PageBreak,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// External link target, when this run is inside an `<a href>`.
    pub link: Option<String>,
    /// An image: the media file name it was written as.
    pub image: Option<String>,
    /// Its size in EMUs, when the document knows it. Without this an exported picture is a guess,
    /// and a guessed logo lands cropped (2026-08-06).
    pub image_emu: Option<(i64, i64)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableOut {
    pub rows: Vec<Vec<CellOut>>,
    /// Column widths in millimetres, one per grid column; empty for auto.
    pub col_widths_mm: Vec<f64>,
    pub bordered: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CellOut {
    pub blocks: Vec<Node>,
    pub colspan: u32,
    pub rowspan: u32,
    /// `#rrggbb` fill, if any.
    pub fill: Option<String>,
    /// True for the continuation cells a rowspan swallows — written as vMerge-continue so the
    /// merge survives the trip.
    pub vmerge_cont: bool,
    pub header: bool,
}

/// Page geometry for the exported section, millimetres.
#[derive(Debug, Clone, Copy)]
pub struct PageOut {
    pub width_mm: f64,
    pub height_mm: f64,
    pub top_mm: f64,
    pub right_mm: f64,
    pub bottom_mm: f64,
    pub left_mm: f64,
}

fn twips(mm: f64) -> i64 {
    (mm / 25.4 * 1440.0).round() as i64
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Build a `.docx` from blocks, media and page geometry.
///
/// `media` maps the file name written into `word/media/` to its bytes; a [`Run::image`] naming a
/// file not in the map is dropped rather than producing a dangling relationship, which is one of
/// the few ways to make Word refuse a file outright.
pub fn write_docx(
    nodes: &[Node],
    media: &HashMap<String, Vec<u8>>,
    page: PageOut,
) -> Result<Vec<u8>, String> {
    // Relationship ids are allocated as the body is written: hyperlinks and images each need one,
    // and they must exist in document.xml.rels or the package is invalid.
    let mut rels: Vec<(String, String, bool)> = Vec::new(); // (id, target, external)
    let mut used_media: Vec<String> = Vec::new();
    let mut body = String::new();

    for node in nodes {
        write_node(node, &mut body, &mut rels, &mut used_media, media);
    }

    // The section: page size, margins, and nothing else. Landscape is expressed by the width and
    // height alone, which is what Word reads anyway.
    body.push_str(&format!(
        r#"<w:sectPr><w:pgSz w:w="{w}" w:h="{h}"/><w:pgMar w:top="{t}" w:right="{r}" w:bottom="{b}" w:left="{l}" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr>"#,
        w = twips(page.width_mm),
        h = twips(page.height_mm),
        t = twips(page.top_mm),
        r = twips(page.right_mm),
        b = twips(page.bottom_mm),
        l = twips(page.left_mm),
    ));

    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><w:body>{body}</w:body></w:document>"#
    );

    let mut rel_xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdStyles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#,
    );
    for (id, target, external) in &rels {
        let kind = if *external {
            "hyperlink"
        } else {
            "image"
        };
        rel_xml.push_str(&format!(
            r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}" Target="{}"{}/>"#,
            xml_escape(target),
            if *external { r#" TargetMode="External""# } else { "" }
        ));
    }
    rel_xml.push_str("</Relationships>");

    // Content types: the defaults cover the parts, plus one per image extension actually used.
    let mut exts: Vec<&str> = Vec::new();
    for name in &used_media {
        if let Some(e) = name.rsplit('.').next() {
            if !exts.contains(&e) {
                exts.push(e);
            }
        }
    }
    let mut types = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>"#,
    );
    for e in &exts {
        let mime = match *e {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "bmp" => "image/bmp",
            "svg" => "image/svg+xml",
            _ => "application/octet-stream",
        };
        types.push_str(&format!(r#"<Default Extension="{e}" ContentType="{mime}"/>"#));
    }
    types.push_str(
        r#"<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#,
    );

    let package_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let opt = zip::write::FileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let mut put = |name: &str, data: &[u8]| -> Result<(), String> {
            z.start_file(name, opt).map_err(|e| e.to_string())?;
            z.write_all(data).map_err(|e| e.to_string())
        };
        put("[Content_Types].xml", types.as_bytes())?;
        put("_rels/.rels", package_rels.as_bytes())?;
        put("word/document.xml", document.as_bytes())?;
        put("word/styles.xml", STYLES.as_bytes())?;
        put("word/_rels/document.xml.rels", rel_xml.as_bytes())?;
        for name in &used_media {
            if let Some(bytes) = media.get(name) {
                put(&format!("word/media/{name}"), bytes)?;
            }
        }
        z.finish().map_err(|e| e.to_string())?;
    }
    Ok(buf.into_inner())
}

fn write_node(
    node: &Node,
    out: &mut String,
    rels: &mut Vec<(String, String, bool)>,
    used_media: &mut Vec<String>,
    media: &HashMap<String, Vec<u8>>,
) {
    match node {
        Node::PageBreak => {
            out.push_str(r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#);
        }
        Node::Para { level, runs, align } => {
            let mut props = String::new();
            if *level >= 1 && *level <= 6 {
                props.push_str(&format!(r#"<w:pStyle w:val="Heading{level}"/>"#));
            }
            if let Some(a) = align {
                let val = match a.as_str() {
                    "center" => "center",
                    "right" => "right",
                    "justify" => "both",
                    _ => "left",
                };
                props.push_str(&format!(r#"<w:jc w:val="{val}"/>"#));
            }
            write_para(out, &props, runs, rels, used_media, media);
        }
        Node::Item { ordered, depth, runs } => {
            // numId 1 = bullets, 2 = decimal, both defined in styles.xml's numbering-free form:
            // Word accepts pStyle ListParagraph with an explicit indent, which is what a reader
            // sees as a list without demanding a numbering.xml part.
            let indent = 360 + 360 * (*depth as i64);
            let marker = if *ordered { "•" } else { "•" };
            let _ = marker;
            let props = format!(
                r#"<w:pStyle w:val="ListParagraph"/><w:numPr><w:ilvl w:val="{depth}"/><w:numId w:val="{}"/></w:numPr><w:ind w:left="{indent}"/>"#,
                if *ordered { 2 } else { 1 }
            );
            write_para(out, &props, runs, rels, used_media, media);
        }
        Node::Table(t) => write_table(t, out, rels, used_media, media),
    }
}

fn write_para(
    out: &mut String,
    props: &str,
    runs: &[Run],
    rels: &mut Vec<(String, String, bool)>,
    used_media: &mut Vec<String>,
    media: &HashMap<String, Vec<u8>>,
) {
    out.push_str("<w:p>");
    if !props.is_empty() {
        out.push_str(&format!("<w:pPr>{props}</w:pPr>"));
    }
    for run in runs {
        write_run(out, run, rels, used_media, media);
    }
    out.push_str("</w:p>");
}

fn write_run(
    out: &mut String,
    run: &Run,
    rels: &mut Vec<(String, String, bool)>,
    used_media: &mut Vec<String>,
    media: &HashMap<String, Vec<u8>>,
) {
    // A picture run: only written when the bytes are actually present, or the package would
    // reference a part that does not exist and Word would refuse the whole file.
    if let Some(name) = &run.image {
        if media.contains_key(name) {
            if !used_media.contains(name) {
                used_media.push(name.clone());
            }
            let id = format!("rIdImg{}", rels.len() + 1);
            rels.push((id.clone(), format!("media/{name}"), false));
            // The document's own dimensions when it has them; otherwise a modest default that
            // keeps the picture visible rather than filling the page.
            out.push_str(&format!(
                r#"<w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{cx}" cy="{cy}"/><wp:docPr id="{n}" name="Picture {n}"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="{n}" name="{name}"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="{id}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#,
                n = rels.len(),
                name = xml_escape(name),
                cx = run.image_emu.map(|(w, _)| w).unwrap_or(1_800_000),
                cy = run.image_emu.map(|(_, h)| h).unwrap_or(1_350_000),
            ));
        }
        return;
    }
    if run.text.is_empty() {
        return;
    }

    let mut rpr = String::new();
    if run.bold {
        rpr.push_str("<w:b/>");
    }
    if run.italic {
        rpr.push_str("<w:i/>");
    }
    if run.underline {
        rpr.push_str(r#"<w:u w:val="single"/>"#);
    }
    if run.strike {
        rpr.push_str("<w:strike/>");
    }
    if run.link.is_some() {
        rpr.push_str(r#"<w:rStyle w:val="Hyperlink"/>"#);
    }
    let rpr = if rpr.is_empty() { String::new() } else { format!("<w:rPr>{rpr}</w:rPr>") };
    // xml:space="preserve" is what keeps a pen-fill field's spaces alive on the way back out —
    // the same whitespace that had to be hardened on the way in.
    let text = format!(
        r#"<w:r>{rpr}<w:t xml:space="preserve">{}</w:t></w:r>"#,
        xml_escape(&run.text)
    );

    match &run.link {
        Some(href) => {
            let id = format!("rIdLnk{}", rels.len() + 1);
            rels.push((id.clone(), href.clone(), true));
            out.push_str(&format!(r#"<w:hyperlink r:id="{id}">{text}</w:hyperlink>"#));
        }
        None => out.push_str(&text),
    }
}

fn write_table(
    t: &TableOut,
    out: &mut String,
    rels: &mut Vec<(String, String, bool)>,
    used_media: &mut Vec<String>,
    media: &HashMap<String, Vec<u8>>,
) {
    out.push_str("<w:tbl><w:tblPr>");
    if t.bordered {
        out.push_str(
            r#"<w:tblBorders><w:top w:val="single" w:sz="4" w:color="000000"/><w:left w:val="single" w:sz="4" w:color="000000"/><w:bottom w:val="single" w:sz="4" w:color="000000"/><w:right w:val="single" w:sz="4" w:color="000000"/><w:insideH w:val="single" w:sz="4" w:color="000000"/><w:insideV w:val="single" w:sz="4" w:color="000000"/></w:tblBorders>"#,
        );
    }
    let total: f64 = t.col_widths_mm.iter().sum();
    if total > 0.0 {
        out.push_str(&format!(
            r#"<w:tblW w:w="{}" w:type="dxa"/>"#,
            twips(total)
        ));
    }
    out.push_str("</w:tblPr>");

    if !t.col_widths_mm.is_empty() {
        out.push_str("<w:tblGrid>");
        for w in &t.col_widths_mm {
            out.push_str(&format!(r#"<w:gridCol w:w="{}"/>"#, twips(*w)));
        }
        out.push_str("</w:tblGrid>");
    }

    for row in &t.rows {
        out.push_str("<w:tr>");
        let mut col = 0usize;
        for cell in row {
            out.push_str("<w:tc><w:tcPr>");
            if let Some(w) = t.col_widths_mm.get(col..col + cell.colspan as usize) {
                let sum: f64 = w.iter().sum();
                if sum > 0.0 {
                    out.push_str(&format!(r#"<w:tcW w:w="{}" w:type="dxa"/>"#, twips(sum)));
                }
            }
            if cell.colspan > 1 {
                out.push_str(&format!(r#"<w:gridSpan w:val="{}"/>"#, cell.colspan));
            }
            if cell.vmerge_cont {
                out.push_str("<w:vMerge/>");
            } else if cell.rowspan > 1 {
                out.push_str(r#"<w:vMerge w:val="restart"/>"#);
            }
            if let Some(fill) = &cell.fill {
                out.push_str(&format!(
                    r#"<w:shd w:val="clear" w:color="auto" w:fill="{}"/>"#,
                    fill.trim_start_matches('#').to_uppercase()
                ));
            }
            out.push_str("</w:tcPr>");
            if cell.blocks.is_empty() {
                out.push_str("<w:p/>");
            } else {
                for b in &cell.blocks {
                    write_node(b, out, rels, used_media, media);
                }
            }
            out.push_str("</w:tc>");
            col += cell.colspan as usize;
        }
        out.push_str("</w:tr>");
    }
    out.push_str("</w:tbl>");
    // A table must not be the last thing in a body: Word requires a trailing paragraph, and
    // without it the document opens with a repair prompt.
    out.push_str("<w:p/>");
}

/// The style part. Heading1-6, ListParagraph and Hyperlink are the styles the body refers to; a
/// document referring to a style that does not exist still opens, but Word renders the heading as
/// body text, which is exactly the fidelity this export exists to keep.
const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri"/><w:sz w:val="22"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="120"/></w:pPr></w:pPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:pPr><w:outlineLvl w:val="0"/><w:spacing w:before="240" w:after="120"/></w:pPr><w:rPr><w:b/><w:sz w:val="40"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:pPr><w:outlineLvl w:val="1"/><w:spacing w:before="200" w:after="100"/></w:pPr><w:rPr><w:b/><w:sz w:val="32"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading3"><w:name w:val="heading 3"/><w:basedOn w:val="Normal"/><w:pPr><w:outlineLvl w:val="2"/></w:pPr><w:rPr><w:b/><w:sz w:val="28"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading4"><w:name w:val="heading 4"/><w:basedOn w:val="Normal"/><w:pPr><w:outlineLvl w:val="3"/></w:pPr><w:rPr><w:b/><w:i/><w:sz w:val="24"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading5"><w:name w:val="heading 5"/><w:basedOn w:val="Normal"/><w:pPr><w:outlineLvl w:val="4"/></w:pPr><w:rPr><w:b/><w:sz w:val="22"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading6"><w:name w:val="heading 6"/><w:basedOn w:val="Normal"/><w:pPr><w:outlineLvl w:val="5"/></w:pPr><w:rPr><w:i/><w:sz w:val="22"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/><w:basedOn w:val="Normal"/><w:pPr><w:ind w:left="720"/><w:contextualSpacing/></w:pPr></w:style>
<w:style w:type="character" w:styleId="Hyperlink"><w:name w:val="Hyperlink"/><w:rPr><w:color w:val="0563C1"/><w:u w:val="single"/></w:rPr></w:style>
</w:styles>"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> PageOut {
        PageOut {
            width_mm: 210.0,
            height_mm: 297.0,
            top_mm: 20.0,
            right_mm: 20.0,
            bottom_mm: 20.0,
            left_mm: 20.0,
        }
    }

    fn run(text: &str) -> Run {
        Run { text: text.into(), ..Run::default() }
    }

    fn read(zipped: &[u8], name: &str) -> String {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(zipped)).unwrap();
        let mut f = z.by_name(name).unwrap();
        let mut s = String::new();
        std::io::Read::read_to_string(&mut f, &mut s).unwrap();
        s
    }

    #[test]
    fn the_package_carries_every_part_word_demands() {
        let d = write_docx(&[Node::Para { level: 0, runs: vec![run("hi")], align: None }], &HashMap::new(), page())
            .unwrap();
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(&d[..])).unwrap();
        let names: Vec<String> = (0..z.len()).map(|i| z.by_index(i).unwrap().name().to_string()).collect();
        for want in [
            "[Content_Types].xml",
            "_rels/.rels",
            "word/document.xml",
            "word/styles.xml",
            "word/_rels/document.xml.rels",
        ] {
            assert!(names.contains(&want.to_string()), "missing {want} in {names:?}");
        }
    }

    #[test]
    fn headings_runs_and_whitespace_survive() {
        let nodes = vec![
            Node::Para { level: 1, runs: vec![run("Title")], align: None },
            Node::Para {
                level: 0,
                runs: vec![
                    Run { text: "bold".into(), bold: true, ..Run::default() },
                    run("  spaced  "),
                ],
                align: Some("center".into()),
            },
        ];
        let d = write_docx(&nodes, &HashMap::new(), page()).unwrap();
        let doc = read(&d, "word/document.xml");
        assert!(doc.contains(r#"<w:pStyle w:val="Heading1"/>"#), "{doc}");
        assert!(doc.contains("<w:b/>"), "{doc}");
        assert!(doc.contains(r#"<w:jc w:val="center"/>"#), "{doc}");
        // The pen-fill spaces come back out intact.
        assert!(doc.contains(r#"<w:t xml:space="preserve">  spaced  </w:t>"#), "{doc}");
    }

    #[test]
    fn tables_carry_merges_widths_and_shading() {
        let cell = |text: &str, colspan: u32, rowspan: u32, fill: Option<&str>, cont: bool| CellOut {
            blocks: vec![Node::Para { level: 0, runs: vec![run(text)], align: None }],
            colspan,
            rowspan,
            fill: fill.map(|f| f.to_string()),
            vmerge_cont: cont,
            header: false,
        };
        let t = TableOut {
            rows: vec![
                vec![cell("wide", 2, 1, Some("#D9D9D9"), false)],
                vec![cell("tall", 1, 2, None, false), cell("b", 1, 1, None, false)],
                vec![cell("", 1, 1, None, true), cell("c", 1, 1, None, false)],
            ],
            col_widths_mm: vec![50.0, 100.0],
            bordered: true,
        };
        let d = write_docx(&[Node::Table(t)], &HashMap::new(), page()).unwrap();
        let doc = read(&d, "word/document.xml");
        assert!(doc.contains(r#"<w:gridSpan w:val="2"/>"#), "{doc}");
        assert!(doc.contains(r#"<w:vMerge w:val="restart"/>"#), "{doc}");
        assert!(doc.contains("<w:vMerge/>"), "{doc}");
        assert!(doc.contains(r#"w:fill="D9D9D9""#), "{doc}");
        assert!(doc.contains("<w:tblBorders>"), "{doc}");
        // 50mm = 2835 twips, 100mm = 5669 (rounded).
        assert!(doc.contains(r#"<w:gridCol w:w="2835"/>"#), "{doc}");
        // A table may not end the body.
        assert!(doc.trim_end().contains("</w:tbl><w:p/>"), "{doc}");
    }

    #[test]
    fn a_link_gets_an_external_relationship_and_a_picture_gets_its_part() {
        let mut media = HashMap::new();
        media.insert("logo.png".to_string(), vec![0x89, b'P', b'N', b'G']);
        let nodes = vec![Node::Para {
            level: 0,
            runs: vec![
                Run { text: "click".into(), link: Some("https://example.com/x".into()), ..Run::default() },
                Run { image: Some("logo.png".into()), ..Run::default() },
                // A picture whose bytes are absent must be dropped, not referenced.
                Run { image: Some("ghost.png".into()), ..Run::default() },
            ],
            align: None,
        }];
        let d = write_docx(&nodes, &media, page()).unwrap();
        let doc = read(&d, "word/document.xml");
        let rels = read(&d, "word/_rels/document.xml.rels");
        assert!(doc.contains("<w:hyperlink"), "{doc}");
        assert!(rels.contains(r#"TargetMode="External""#), "{rels}");
        assert!(rels.contains("media/logo.png"), "{rels}");
        assert!(!rels.contains("ghost.png"), "{rels}");
        let types = read(&d, "[Content_Types].xml");
        assert!(types.contains(r#"Extension="png""#), "{types}");
        // The bytes are really in there (binary, so read them as bytes — not as text).
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(&d[..])).unwrap();
        let mut f = z.by_name("word/media/logo.png").unwrap();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut bytes).unwrap();
        assert_eq!(bytes, vec![0x89, b'P', b'N', b'G']);
    }

    #[test]
    fn the_section_carries_the_page_geometry() {
        let d = write_docx(&[], &HashMap::new(), page()).unwrap();
        let doc = read(&d, "word/document.xml");
        // A4 = 11906 x 16838 twips, 20mm margins = 1134.
        assert!(doc.contains(r#"<w:pgSz w:w="11906" w:h="16838"/>"#), "{doc}");
        assert!(doc.contains(r#"w:top="1134""#), "{doc}");
    }
}
