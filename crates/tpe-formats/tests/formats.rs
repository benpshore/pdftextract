//! End-to-end tests over fixtures built in memory: OOXML packages zipped from
//! hand-written XML, inline csv/html/md/txt, a synthetic Pages package, and
//! the audio path with no engine on PATH.

use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;

use tpe_formats::{Format, FormatsError, Options, Status, audio, extract_path, write_outputs};
use zip::write::SimpleFileOptions;

fn zip_bytes(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, body) in entries {
        writer.start_file(*name, options).unwrap();
        writer.write_all(body.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn write_fixture(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = dir.join(name);
    fs::write(&path, bytes).unwrap();
    path
}

fn extract(dir: &Path, name: &str, bytes: &[u8]) -> tpe_formats::FormatsResult {
    let path = write_fixture(dir, name, bytes);
    let before = fs::read(&path).unwrap();
    let result = extract_path(&path, &Options::default()).unwrap();
    assert_eq!(fs::read(&path).unwrap(), before, "input must not change");
    result
}

const CORE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
<dc:title>Fixture Title</dc:title><dc:creator>A. Author</dc:creator><cp:keywords>k1; k2</cp:keywords>
<dcterms:created xsi:type="dcterms:W3CDTF">2026-01-02T03:04:05Z</dcterms:created></cp:coreProperties>"#;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

fn docx_fixture() -> Vec<u8> {
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="{W}"><w:body>
<w:p><w:pPr><w:pStyle w:val="Title"/></w:pPr><w:r><w:t>Doc Title</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Intro</w:t></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">Hello </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>world</w:t></w:r><w:r><w:tab/><w:t>tabbed</w:t></w:r><w:r><w:footnoteReference w:id="1"/></w:r></w:p>
<w:p><w:r><w:t>Kept</w:t></w:r><w:del><w:r><w:delText>DELETED</w:delText></w:r></w:del><w:ins><w:r><w:t> inserted</w:t></w:r></w:ins></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="1"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>item</w:t></w:r></w:p>
<w:p><w:hyperlink r:id="rId9" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:r><w:t>linked</w:t></w:r></w:hyperlink></w:p>
<w:tbl><w:tr><w:tc><w:p><w:r><w:t>h1</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>h2</w:t></w:r></w:p></w:tc></w:tr>
<w:tr><w:tc><w:p><w:r><w:t>a</w:t></w:r></w:p><w:p><w:r><w:t>b</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr><w:p><w:r><w:t>c</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
<w:p/>
<w:sectPr/></w:body></w:document>"#
    );
    let styles = format!(
        r#"<w:styles xmlns:w="{W}"><w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/></w:style></w:styles>"#
    );
    let header =
        format!(r#"<w:hdr xmlns:w="{W}"><w:p><w:r><w:t>Running head</w:t></w:r></w:p></w:hdr>"#);
    let footer =
        format!(r#"<w:ftr xmlns:w="{W}"><w:p><w:r><w:t>Page footer</w:t></w:r></w:p></w:ftr>"#);
    let footnotes = format!(
        r#"<w:footnotes xmlns:w="{W}"><w:footnote w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:footnote><w:footnote w:id="1"><w:p><w:r><w:t>A footnote.</w:t></w:r></w:p></w:footnote></w:footnotes>"#
    );
    zip_bytes(&[
        ("[Content_Types].xml", "<Types/>"),
        ("word/document.xml", &document),
        ("word/styles.xml", &styles),
        ("word/header1.xml", &header),
        ("word/footer1.xml", &footer),
        ("word/footnotes.xml", &footnotes),
        ("docProps/core.xml", CORE),
    ])
}

#[test]
fn docx_paragraphs_runs_tables_headers_footnotes() {
    let dir = tempfile::tempdir().unwrap();
    let result = extract(dir.path(), "fixture.docx", &docx_fixture());
    assert_eq!(result.format, Format::Docx);
    assert_eq!(result.status, Status::Complete, "{:?}", result.warnings);
    assert_eq!(result.title.as_deref(), Some("Fixture Title"));
    assert_eq!(
        result.metadata.get("creator").map(String::as_str),
        Some("A. Author")
    );
    assert_eq!(result.document.hash.len(), 64);
    let body = &result.sections[0];
    assert_eq!(body.kind, "body");
    let kinds: Vec<(&str, &str)> = body
        .blocks
        .iter()
        .map(|b| (b.kind.as_str(), b.text.as_str()))
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("heading", "Doc Title"),
            ("heading", "Intro"),
            ("paragraph", "Hello world\ttabbed[1]"),
            ("paragraph", "Kept inserted"),
            ("list_item", "item"),
            ("paragraph", "linked"),
            ("table", "h1\th2\na\nb\tc\t"),
        ]
    );
    assert_eq!(body.blocks[0].level, Some(0));
    assert_eq!(body.blocks[1].level, Some(1));
    assert_eq!(body.blocks[4].level, Some(1));
    assert_eq!(
        body.blocks[6].rows.as_ref().unwrap()[1],
        vec!["a\nb", "c", ""]
    );
    assert_eq!(result.sections[1].kind, "header");
    assert_eq!(result.sections[1].blocks[0].text, "Running head");
    assert_eq!(result.sections[2].kind, "footer");
    assert_eq!(result.notes.len(), 1);
    assert_eq!(result.notes[0].kind, "footnote");
    assert_eq!(result.notes[0].anchor.as_deref(), Some("1"));
    assert_eq!(result.notes[0].text, "A footnote.");
    assert!(
        result
            .text
            .contains("Hello world\ttabbed[1]\n\nKept inserted")
    );
    assert!(result.text.ends_with("[footnote 1]\nA footnote.\n"));
    assert!(!result.text.contains("DELETED"));
}

const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn pptx_fixture() -> Vec<u8> {
    let presentation = format!(
        r#"<p:presentation xmlns:p="{P}" xmlns:r="{R}"><p:sldIdLst><p:sldId id="257" r:id="rId3"/><p:sldId id="256" r:id="rId2"/></p:sldIdLst></p:presentation>"#
    );
    let presentation_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="x/slide" Target="slides/slide1.xml"/><Relationship Id="rId3" Type="x/slide" Target="slides/slide2.xml"/></Relationships>"#;
    let slide = |title: &str, body: &str| {
        format!(
            r#"<p:sld xmlns:p="{P}" xmlns:a="{A}"><p:cSld><p:spTree>
<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title"/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>{title}</a:t></a:r></a:p></p:txBody></p:sp>
<p:sp><p:nvSpPr><p:cNvPr id="3" name="Body"/><p:nvPr><p:ph idx="1"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>{body}</a:t></a:r></a:p><a:p><a:pPr lvl="1"/><a:r><a:t>sub </a:t></a:r><a:r><a:t>point</a:t></a:r></a:p></p:txBody></p:sp>
<p:sp><p:nvSpPr><p:cNvPr id="4" name="Num"/><p:nvPr><p:ph type="sldNum"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:fld type="slidenum"><a:t>1</a:t></a:fld></a:p></p:txBody></p:sp>
<p:graphicFrame><a:graphic><a:graphicData><a:tbl><a:tr><a:tc><a:txBody><a:p><a:r><a:t>c1</a:t></a:r></a:p></a:txBody></a:tc><a:tc><a:txBody><a:p><a:r><a:t>c2</a:t></a:r></a:p></a:txBody></a:tc></a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame>
</p:spTree></p:cSld></p:sld>"#
        )
    };
    let slide1 = slide("First", "Body one");
    let slide2 = slide("Second", "Body two");
    let slide2_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="x/notesSlide" Target="../notesSlides/notesSlide1.xml"/></Relationships>"#;
    let notes = format!(
        r#"<p:notes xmlns:p="{P}" xmlns:a="{A}"><p:cSld><p:spTree><p:sp><p:nvSpPr><p:cNvPr id="2" name="Slide Image"/><p:nvPr><p:ph type="sldImg"/></p:nvPr></p:nvSpPr></p:sp><p:sp><p:nvSpPr><p:cNvPr id="3" name="Notes"/><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>Remember to pause.</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:notes>"#
    );
    zip_bytes(&[
        ("[Content_Types].xml", "<Types/>"),
        ("ppt/presentation.xml", &presentation),
        ("ppt/_rels/presentation.xml.rels", presentation_rels),
        ("ppt/slides/slide1.xml", &slide1),
        ("ppt/slides/slide2.xml", &slide2),
        ("ppt/slides/_rels/slide2.xml.rels", slide2_rels),
        ("ppt/notesSlides/notesSlide1.xml", &notes),
        ("docProps/core.xml", CORE),
    ])
}

#[test]
fn pptx_slide_order_frames_tables_notes() {
    let dir = tempfile::tempdir().unwrap();
    let result = extract(dir.path(), "deck.pptx", &pptx_fixture());
    assert_eq!(result.format, Format::Pptx);
    assert_eq!(result.status, Status::Complete, "{:?}", result.warnings);
    assert_eq!(result.sections.len(), 2);
    // sldIdLst order puts slide2 first.
    assert_eq!(result.sections[0].title.as_deref(), Some("Second"));
    assert_eq!(result.sections[1].title.as_deref(), Some("First"));
    let blocks = &result.sections[0].blocks;
    assert_eq!(blocks[0].text, "Body two");
    assert_eq!(blocks[1].kind, "list_item");
    assert_eq!(blocks[1].text, "sub point");
    assert_eq!(blocks[2].kind, "table");
    assert_eq!(blocks[2].rows.as_ref().unwrap()[0], vec!["c1", "c2"]);
    assert!(
        !result.text.contains("\n1\n"),
        "slide number placeholder dropped"
    );
    assert_eq!(result.notes.len(), 1);
    assert_eq!(result.notes[0].kind, "speaker_notes");
    assert_eq!(result.notes[0].anchor.as_deref(), Some("slide 1"));
    assert_eq!(result.notes[0].text, "Remember to pause.");
    assert!(result.text.starts_with("Second\n\nBody two"));
}

fn xlsx_fixture() -> Vec<u8> {
    let workbook = format!(
        r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="{R}"><workbookPr/><sheets><sheet name="Data" sheetId="1" r:id="rId1"/><sheet name="Hidden" sheetId="2" state="hidden" r:id="rId2"/></sheets></workbook>"#
    );
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="x/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="x/worksheet" Target="worksheets/sheet2.xml"/><Relationship Id="rId3" Type="x/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#;
    let shared = r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="2" uniqueCount="2"><si><t>Name</t></si><si><r><t>Rich </t></r><r><rPr><b/></rPr><t>text</t></r><rPh sb="0" eb="1"><t>ignored</t></rPh></si></sst>"#;
    let styles = r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts count="1"><numFmt numFmtId="164" formatCode="yyyy-mm-dd"/></numFmts><cellXfs count="5"><xf numFmtId="0"/><xf numFmtId="164"/><xf numFmtId="10"/><xf numFmtId="4"/><xf numFmtId="12"/></cellXfs></styleSheet>"#;
    let sheet1 = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
<row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="inlineStr"><is><t>Inline</t></is></c><c r="C1" t="s"><v>1</v></c></row>
<row r="2"><c r="A2"><v>3</v></c><c r="B2" s="1"><v>45658</v></c><c r="C2" s="2"><v>0.256</v></c><c r="D2" s="3"><v>1234.5</v></c></row>
<row r="4"><c r="A4"><f>A2*2</f><v>6</v></c><c r="B4" t="b"><v>1</v></c><c r="C4" t="e"><v>#DIV/0!</v></c><c r="D4" t="str"><f>CONCAT("a","b")</f><v>ab</v></c><c r="E4"><f>NOW()</f></c><c r="F4" s="4"><v>0.5</v></c></row>
</sheetData></worksheet>"#;
    let sheet2 = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>7</v></c></row></sheetData></worksheet>"#;
    zip_bytes(&[
        ("[Content_Types].xml", "<Types/>"),
        ("xl/workbook.xml", &workbook),
        ("xl/_rels/workbook.xml.rels", rels),
        ("xl/sharedStrings.xml", shared),
        ("xl/styles.xml", styles),
        ("xl/worksheets/sheet1.xml", sheet1),
        ("xl/worksheets/sheet2.xml", sheet2),
    ])
}

#[test]
fn xlsx_sheets_strings_formats_formulas() {
    let dir = tempfile::tempdir().unwrap();
    let result = extract(dir.path(), "book.xlsx", &xlsx_fixture());
    assert_eq!(result.format, Format::Xlsx);
    assert_eq!(result.status, Status::Partial, "{:?}", result.warnings);
    assert_eq!(result.sections.len(), 2);
    assert_eq!(result.sections[0].title.as_deref(), Some("Data"));
    let rows = result.sections[0].blocks[0].rows.as_ref().unwrap();
    assert_eq!(rows[0], vec!["Name", "Inline", "Rich text"]);
    assert_eq!(rows[1], vec!["3", "2025-01-01", "25.60%", "1,234.50"]);
    assert!(rows[2].is_empty(), "row 3 is a gap");
    assert_eq!(rows[3], vec!["6", "TRUE", "#DIV/0!", "ab", "", "0.5"]);
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.contains("Data!E4: formula without a cached value"))
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.contains("number format \"# ?/?\" not rendered"))
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.contains("sheet 'Hidden' is hidden"))
    );
    assert_eq!(
        result.sections[1].blocks[0].rows.as_ref().unwrap()[0],
        vec!["7"]
    );
    assert!(
        result.text.contains(
            "Data\n\nName\tInline\tRich text\n3\t2025-01-01\t25.60%\t1,234.50\n\n6\tTRUE"
        )
    );
}

#[test]
fn csv_sniffing_quoting_bom_and_encoding() {
    let dir = tempfile::tempdir().unwrap();
    let result = extract(
        dir.path(),
        "semi.csv",
        "\u{feff}a;b;c\n1;\"x;y\";\"q\"\"d\"\n".as_bytes(),
    );
    assert_eq!(result.format, Format::Csv);
    assert_eq!(
        result.metadata.get("delimiter").map(String::as_str),
        Some(";")
    );
    assert_eq!(
        result.metadata.get("encoding").map(String::as_str),
        Some("utf-8-bom")
    );
    let rows = result.sections[0].blocks[0].rows.as_ref().unwrap();
    assert_eq!(rows[1], vec!["1", "x;y", "q\"d"]);

    let latin = extract(dir.path(), "latin.csv", b"name,city\nJos\xe9,M\xfcnchen\n");
    assert_eq!(
        latin.metadata.get("encoding").map(String::as_str),
        Some("windows-1252")
    );
    let rows = latin.sections[0].blocks[0].rows.as_ref().unwrap();
    assert_eq!(rows[1], vec!["José", "München"]);
    assert!(latin.warnings.iter().any(|w| w.contains("windows-1252")));

    let tsv = extract(dir.path(), "t.tsv", b"a\tb\n1,5\t2\n");
    assert_eq!(tsv.format, Format::Tsv);
    assert_eq!(
        tsv.sections[0].blocks[0].rows.as_ref().unwrap()[1],
        vec!["1,5", "2"]
    );

    let utf16 = extract(
        dir.path(),
        "u.csv",
        &[0xFF, 0xFE, b'a', 0, b',', 0, b'b', 0, b'\n', 0],
    );
    assert_eq!(
        utf16.metadata.get("encoding").map(String::as_str),
        Some("utf-16le")
    );
    assert_eq!(
        utf16.sections[0].blocks[0].rows.as_ref().unwrap()[0],
        vec!["a", "b"]
    );
}

#[test]
fn html_structure_and_entities() {
    let dir = tempfile::tempdir().unwrap();
    let html = "<!DOCTYPE html><html lang=\"de\"><head><title>Seite &amp; Titel</title><script>var x = '<p>';</script><style>p{}</style><meta name=\"description\" content=\"desc\"></head><body><h1>Head</h1><p>Caf&eacute; &#8212; done<br>next</p><ol><li>one</li><li>two</li></ol><table><tr><td>a</td><td>b</td></tr></table><!-- c --></body></html>";
    let result = extract(dir.path(), "page.html", html.as_bytes());
    assert_eq!(result.format, Format::Html);
    assert_eq!(result.status, Status::Complete, "{:?}", result.warnings);
    assert_eq!(result.title.as_deref(), Some("Seite & Titel"));
    assert_eq!(result.metadata.get("lang").map(String::as_str), Some("de"));
    assert_eq!(
        result.metadata.get("meta.description").map(String::as_str),
        Some("desc")
    );
    let kinds: Vec<(&str, &str)> = result.sections[0]
        .blocks
        .iter()
        .map(|b| (b.kind.as_str(), b.text.as_str()))
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("heading", "Head"),
            ("paragraph", "Café — done\nnext"),
            ("list_item", "one"),
            ("list_item", "two"),
            ("table", "a\tb"),
        ]
    );
    assert!(!result.text.contains("var x"));
    // Sniffed without an extension.
    let sniffed = extract(dir.path(), "noext", b"  <!doctype HTML><p>hi</p>");
    assert_eq!(sniffed.format, Format::Html);
    assert_eq!(sniffed.text, "hi\n");
}

#[test]
fn markdown_front_matter_and_normalisation() {
    let dir = tempfile::tempdir().unwrap();
    let md = "---\ntitle: Notes\nauthor: Me\n---\r\n\r\n# Heading  \r\n\r\nSome *emph* text\r\nwith [a link](http://x.y).\r\n\r\n- one\n- two\n\n```\ncode here\n```\n";
    let result = extract(dir.path(), "notes.md", md.as_bytes());
    assert_eq!(result.format, Format::Markdown);
    assert_eq!(result.title.as_deref(), Some("Notes"));
    assert_eq!(
        result.metadata.get("author").map(String::as_str),
        Some("Me")
    );
    assert_eq!(result.sections[0].kind, "front_matter");
    assert_eq!(
        result.sections[0].blocks[0].text,
        "title: Notes\nauthor: Me"
    );
    let body = &result.sections[1];
    let kinds: Vec<(&str, &str)> = body
        .blocks
        .iter()
        .map(|b| (b.kind.as_str(), b.text.as_str()))
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("heading", "Heading"),
            ("paragraph", "Some emph text with a link."),
            ("list_item", "one"),
            ("list_item", "two"),
            ("code", "code here"),
        ]
    );
    assert!(!result.text.contains('\r'));
}

#[test]
fn plain_text_paragraphs() {
    let dir = tempfile::tempdir().unwrap();
    let result = extract(
        dir.path(),
        "a.txt",
        b"line one\r\nline two\r\n\r\n\r\npara two  \n",
    );
    assert_eq!(result.format, Format::Text);
    let texts: Vec<&str> = result.sections[0]
        .blocks
        .iter()
        .map(|b| b.text.as_str())
        .collect();
    assert_eq!(texts, vec!["line one\nline two", "para two"]);
    assert_eq!(result.text, "line one\nline two\n\npara two\n");
    let empty = extract(dir.path(), "empty.txt", b"");
    assert_eq!(empty.status, Status::Partial);
}

#[test]
fn pages_package_is_reported_unsupported_with_reason_and_preview_copied() {
    let dir = tempfile::tempdir().unwrap();
    let pages = zip_bytes(&[
        ("Index/Document.iwa", "\x00\x05\x00\x00not-really-iwa"),
        ("Index/DocumentStylesheet.iwa", "x"),
        (
            "Metadata/DocumentIdentifier",
            "4C8D5F2A-0000-0000-0000-000000000000\n",
        ),
        (
            "Metadata/BuildVersionHistory.plist",
            "<?xml version=\"1.0\"?><plist version=\"1.0\"><array><string>Template: Blank (14.0)</string><string>M14.0.1-7040.0.73-4</string></array></plist>",
        ),
        ("preview.pdf", "%PDF-1.4\n%fixture\n"),
    ]);
    let result = extract(dir.path(), "letter.pages", &pages);
    assert_eq!(result.format, Format::Pages);
    assert_eq!(result.status, Status::Unsupported);
    assert!(
        result.warnings[0].starts_with("unsupported: Pages text is stored in Index/*.iwa"),
        "{:?}",
        result.warnings
    );
    assert_eq!(
        result.metadata.get("template").map(String::as_str),
        Some("Blank (14.0)")
    );
    assert_eq!(
        result.metadata.get("build_versions").map(String::as_str),
        Some("M14.0.1-7040.0.73-4")
    );
    assert_eq!(
        result
            .metadata
            .get("document_identifier")
            .map(String::as_str),
        Some("4C8D5F2A-0000-0000-0000-000000000000")
    );
    assert_eq!(
        result.metadata.get("package.iwa_files").map(String::as_str),
        Some("2")
    );
    assert_eq!(result.extra_files.len(), 1);
    let out = dir.path().join("out");
    let outputs = write_outputs(&result, &out, "letter", false)
        .unwrap()
        .unwrap();
    assert!(outputs.txt.is_none(), "no .txt for an unsupported input");
    assert_eq!(outputs.extra, vec![out.join("letter.preview.pdf")]);
    assert_eq!(
        fs::read(out.join("letter.preview.pdf")).unwrap(),
        b"%PDF-1.4\n%fixture\n"
    );
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&outputs.json).unwrap()).unwrap();
    assert_eq!(json["status"], "unsupported");
    assert_eq!(json["format"], "pages");

    // A directory bundle (macOS package form) and an old-format Numbers package.
    let bundle = dir.path().join("sheet.numbers");
    fs::create_dir_all(bundle.join("Index")).unwrap();
    fs::write(bundle.join("Index/Document.iwa"), b"\x00\x01\x00\x00x").unwrap();
    let result = extract_path(&bundle, &Options::default()).unwrap();
    assert_eq!(result.format, Format::Numbers);
    assert_eq!(result.status, Status::Unsupported);
    assert!(result.warnings[0].contains("Numbers text is stored in Index/*.iwa"));
    assert_eq!(
        result.metadata.get("preview_pdf").map(String::as_str),
        Some("absent")
    );
    let old = zip_bytes(&[("index.xml.gz", "zz"), ("QuickLook/Thumbnail.jpg", "jpg")]);
    let result = extract(dir.path(), "old.numbers", &old);
    assert!(
        result.warnings[0].contains("has no Index/Document.iwa"),
        "{:?}",
        result.warnings
    );
}

#[test]
fn audio_without_engine_is_unsupported_here() {
    let dir = tempfile::tempdir().unwrap();
    // A 0.5 s silent 16 kHz mono 16-bit WAV.
    let samples = 8000u32;
    let data_len = samples * 2;
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&16000u32.to_le_bytes());
    wav.extend_from_slice(&32000u32.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.resize(wav.len() + data_len as usize, 0);
    let path = write_fixture(dir.path(), "silence.wav", &wav);
    // The present engine (if any) is deliberately not visible: an empty PATH
    // and no model variables. This is the path a machine without whisper takes.
    let empty = dir.path().join("empty-path");
    fs::create_dir_all(&empty).unwrap();
    let env = audio::EngineEnv {
        path: Some(empty.clone().into_os_string()),
        home: Some(empty.clone()),
        ..audio::EngineEnv::default()
    };
    let detected = audio::detect(&env);
    assert!(detected.is_err());
    let options = Options { audio: Some(env) };
    let result = extract_path(&path, &options).unwrap();
    assert_eq!(result.format, Format::Audio);
    assert_eq!(result.status, Status::Unsupported);
    assert!(
        result.warnings[0].starts_with(
            "unsupported: unsupported here: no local speech engine. whisper.cpp not found: install"
        ),
        "{:?}",
        result.warnings
    );
    assert!(result.warnings[0].contains("openai-whisper not found"));
    assert_eq!(
        result.metadata.get("duration_seconds").map(String::as_str),
        Some("0.50")
    );
    assert_eq!(
        result.metadata.get("sample_rate").map(String::as_str),
        Some("16000")
    );
    // Engine present but no model: still unsupported, says which model to set.
    let bin_dir = dir.path().join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    fs::write(bin_dir.join("whisper-cli"), b"#!/bin/sh\nexit 0\n").unwrap();
    let env = audio::EngineEnv {
        path: Some(bin_dir.clone().into_os_string()),
        home: Some(empty.clone()),
        ..audio::EngineEnv::default()
    };
    let reason = audio::detect(&env).unwrap_err();
    assert!(
        reason.contains("no GGML model: set TPE_WHISPER_MODEL"),
        "{reason}"
    );
    // With a model file the engine is selected without running anything.
    let model = dir.path().join("ggml-tiny.bin");
    fs::write(&model, b"not a model").unwrap();
    let env = audio::EngineEnv {
        path: Some(bin_dir.into_os_string()),
        whisper_model: Some(model.clone()),
        home: Some(empty),
        ..audio::EngineEnv::default()
    };
    match audio::detect(&env).unwrap() {
        audio::Engine::WhisperCpp { model: m, .. } => assert_eq!(m, model),
        other @ audio::Engine::OpenAiWhisper { .. } => panic!("unexpected engine {other:?}"),
    }
    let real = audio::detect(&audio::EngineEnv::from_process());
    if let Ok(engine) = real {
        eprintln!(
            "skipping live transcription check: a local engine is installed ({})",
            engine.label()
        );
    }
}

#[test]
fn outputs_are_written_once_unless_forced() {
    let dir = tempfile::tempdir().unwrap();
    let result = extract(dir.path(), "a.txt", b"hello");
    let out = dir.path().join("out");
    let first = write_outputs(&result, &out, "a", false).unwrap().unwrap();
    assert_eq!(
        fs::read_to_string(first.txt.as_ref().unwrap()).unwrap(),
        "hello\n"
    );
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&first.json).unwrap()).unwrap();
    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["backend"]["name"], "tpe-formats");
    assert_eq!(json["sections"][0]["blocks"][0]["kind"], "paragraph");
    assert!(write_outputs(&result, &out, "a", false).unwrap().is_none());
    assert!(write_outputs(&result, &out, "a", true).unwrap().is_some());
}

#[test]
fn unknown_and_malformed_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_fixture(dir.path(), "blob.bin", &[0u8, 159, 146, 150, 255]);
    match extract_path(&path, &Options::default()) {
        Err(FormatsError::Unsupported(reason)) => assert!(reason.contains("no decoder")),
        other => panic!("expected unsupported, got {other:?}"),
    }
    let path = write_fixture(dir.path(), "broken.docx", b"PK\x03\x04 not a zip");
    assert!(matches!(
        extract_path(&path, &Options::default()),
        Err(FormatsError::Container(_))
    ));
    let no_body = zip_bytes(&[("word/document.xml", "<w:document xmlns:w=\"x\"/>")]);
    let path = write_fixture(dir.path(), "nobody.docx", &no_body);
    assert!(matches!(
        extract_path(&path, &Options::default()),
        Err(FormatsError::Invalid(_))
    ));
    let plain_dir = dir.path().join("plain");
    fs::create_dir_all(&plain_dir).unwrap();
    assert!(matches!(
        extract_path(&plain_dir, &Options::default()),
        Err(FormatsError::Invalid(_))
    ));
}

#[test]
fn cli_exit_codes_and_outputs() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = dir.path().join("in");
    fs::create_dir_all(inputs.join("nested")).unwrap();
    fs::write(inputs.join("a.txt"), b"alpha").unwrap();
    fs::write(inputs.join("nested/a.txt"), b"beta").unwrap();
    fs::write(inputs.join("deck.pptx"), pptx_fixture()).unwrap();
    fs::write(inputs.join("skip.unknownext"), b"zzz").unwrap();
    let out = dir.path().join("out");
    let exe = env!("CARGO_BIN_EXE_tpe-formats");
    let run = |args: &[&str]| std::process::Command::new(exe).args(args).output().unwrap();

    let status = run(&[inputs.to_str().unwrap(), "--out", out.to_str().unwrap()]);
    assert_eq!(
        status.status.code(),
        Some(2),
        "directory without --recursive is a usage error"
    );

    let ok = run(&[
        inputs.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
        "--recursive",
        "--json",
    ]);
    assert_eq!(
        ok.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&ok.stdout).unwrap();
    let reports = report.as_array().unwrap();
    assert_eq!(reports.len(), 3, "{report}");
    assert!(out.join("a.txt").exists() && out.join("a.json").exists());
    assert!(out.join("a-2.txt").exists(), "stem collision gets a suffix");
    assert!(out.join("deck.txt").exists());
    assert_eq!(fs::read_to_string(out.join("a.txt")).unwrap(), "alpha\n");
    assert_eq!(fs::read_to_string(out.join("a-2.txt")).unwrap(), "beta\n");

    let again = run(&[
        inputs.join("a.txt").to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(again.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&again.stdout).contains("skipped"));

    fs::write(
        inputs.join("letter.pages"),
        zip_bytes(&[("Index/Document.iwa", "x")]),
    )
    .unwrap();
    let unsupported = run(&[
        inputs.join("letter.pages").to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(unsupported.status.code(), Some(3));
    assert!(out.join("letter.json").exists());
    assert!(!out.join("letter.txt").exists());

    fs::write(inputs.join("bad.docx"), b"not a zip at all").unwrap();
    let failed = run(&[
        inputs.join("bad.docx").to_str().unwrap(),
        inputs.join("letter.pages").to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
        "--force",
    ]);
    assert_eq!(
        failed.status.code(),
        Some(1),
        "a failure outranks unsupported"
    );
    let missing = run(&[
        dir.path().join("nope.txt").to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(missing.status.code(), Some(2));
}
