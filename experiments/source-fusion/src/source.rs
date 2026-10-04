//! Restricted source inspection. Source CMaps and embedded font cmap agreement
//! corroborate declared Unicode; this does not independently recognize pixels.

use lopdf::{Dictionary, Document, Object, ObjectId, Stream, content::Content};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    sync::atomic::{AtomicBool, Ordering},
};
use tpe_region_evidence::{Digest, SourceIdentity};

#[derive(Clone, Debug)]
pub struct SourceLimits {
    pub max_source_bytes: u64,
    pub max_objects: usize,
    pub max_content_bytes: usize,
    pub max_cmap_bytes: usize,
    pub max_cmap_entries: usize,
    pub max_operators: usize,
    pub max_text_shows: usize,
    pub max_text_bytes: usize,
    pub max_font_bytes: usize,
    pub max_font_cmap_probes: usize,
    pub max_total_font_bytes: usize,
}
impl Default for SourceLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: u64::MAX,
            max_objects: 10_000,
            max_content_bytes: 1024 * 1024,
            max_cmap_bytes: 1024 * 1024,
            max_cmap_entries: 65_536,
            max_operators: 10_000,
            max_text_shows: 1024,
            max_text_bytes: 1024 * 1024,
            max_font_bytes: 4 * 1024 * 1024,
            max_font_cmap_probes: 1_000_000,
            max_total_font_bytes: 8 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceOutcome {
    Supported,
    Unsupported,
    ResourceLimit,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct SourceBox {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SourceOrigin {
    pub page_object: ObjectId,
    pub content_object: ObjectId,
    pub content_sha256: Digest,
    /// Zero-based position in the strictly decoded executed operation list.
    /// This is deliberately not a claimed byte offset.
    pub operator_ordinal: u32,
    pub raw_codes_hex: String,
    pub font_resource: String,
    pub font_object: ObjectId,
    pub descendant_font_object: ObjectId,
    pub encoding_object: Option<ObjectId>,
    pub encoding_sha256: Digest,
    pub to_unicode_object: ObjectId,
    pub to_unicode_sha256: Digest,
    pub descriptor_object: ObjectId,
    pub font_program_object: ObjectId,
    pub font_program_sha256: Digest,
    pub text_matrix: [f64; 6],
    pub font_size: f64,
    pub explicit_widths: Vec<f64>,
    pub declared_font_bbox: SourceBox,
    pub embedded_font_global_bbox: SourceBox,
    pub geometry_font_bbox: SourceBox,
    pub character_codes: Vec<u16>,
    pub glyph_ids: Vec<u16>,
    pub unicode_scalars: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SourceTextWitness {
    pub id: String,
    pub region: SourceBox,
    pub unicode: String,
    pub origin: SourceOrigin,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SourcePage {
    pub page: u32,
    pub width: f64,
    pub height: f64,
    pub rotation: i32,
    pub witnesses: Vec<SourceTextWitness>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SourceIssue {
    pub operator_ordinal: Option<u32>,
    pub raw_codes_hex: Option<String>,
    pub font_resource: Option<String>,
    pub reason: String,
}

/// Only `verify_source` can create this non-deserializable computed value.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct VerifiedSource {
    source: SourceIdentity,
    outcome: SourceOutcome,
    pages: Vec<SourcePage>,
    issues: Vec<SourceIssue>,
}
impl VerifiedSource {
    pub fn source(&self) -> &SourceIdentity {
        &self.source
    }
    pub fn outcome(&self) -> &SourceOutcome {
        &self.outcome
    }
    pub fn pages(&self) -> &[SourcePage] {
        &self.pages
    }
    pub fn issues(&self) -> &[SourceIssue] {
        &self.issues
    }
}

// Verifier implementation follows below. Every rejected executed text show
// invalidates this page's witness set, including earlier successful shows.

type Checked<T> = Result<T, Failure>;
struct Failure {
    outcome: SourceOutcome,
    issue: SourceIssue,
}
fn unsupported(reason: impl Into<String>) -> Failure {
    Failure {
        outcome: SourceOutcome::Unsupported,
        issue: SourceIssue {
            operator_ordinal: None,
            raw_codes_hex: None,
            font_resource: None,
            reason: reason.into(),
        },
    }
}
fn require(condition: bool, reason: &str) -> Checked<()> {
    if condition {
        Ok(())
    } else {
        Err(unsupported(reason))
    }
}
fn bound(condition: bool, reason: &str) -> Checked<()> {
    if condition {
        Ok(())
    } else {
        let mut error = unsupported(reason);
        error.outcome = SourceOutcome::ResourceLimit;
        Err(error)
    }
}
fn cancelled(flag: &AtomicBool) -> Checked<()> {
    if flag.load(Ordering::Acquire) {
        let mut error = unsupported("source inspection cancelled");
        error.outcome = SourceOutcome::Cancelled;
        Err(error)
    } else {
        Ok(())
    }
}
fn pdf<T>(result: lopdf::Result<T>, what: &str) -> Checked<T> {
    result.map_err(|_| unsupported(what))
}
fn obj(doc: &Document, id: ObjectId) -> Checked<&Object> {
    pdf(doc.get_object(id), "unresolved PDF object reference")
}
fn dict(doc: &Document, id: ObjectId) -> Checked<&Dictionary> {
    pdf(obj(doc, id)?.as_dict(), "expected dictionary object")
}
fn key<'a>(dict: &'a Dictionary, name: &[u8]) -> Checked<&'a Object> {
    pdf(dict.get(name), "required source dictionary field missing")
}
fn reference(dict: &Dictionary, name: &[u8]) -> Checked<ObjectId> {
    pdf(
        key(dict, name)?.as_reference(),
        "required indirect object identity missing",
    )
}
fn name(object: &Object) -> Checked<&[u8]> {
    pdf(object.as_name(), "expected PDF name")
}
fn number(object: &Object) -> Checked<f64> {
    let value = match object {
        Object::Integer(value) => *value as f64,
        Object::Real(value) => f64::from(*value),
        _ => return Err(unsupported("expected finite PDF number")),
    };
    require(value.is_finite(), "nonfinite PDF number")?;
    Ok(value)
}
fn rectangle(object: &Object) -> Checked<SourceBox> {
    let values = pdf(object.as_array(), "expected four-coordinate rectangle")?;
    require(values.len() == 4, "rectangle must have four coordinates")?;
    let bbox = SourceBox {
        x0: number(&values[0])?,
        y0: number(&values[1])?,
        x1: number(&values[2])?,
        y1: number(&values[3])?,
    };
    require(
        bbox.x0 < bbox.x1 && bbox.y0 < bbox.y1,
        "nonpositive rectangle",
    )?;
    Ok(bbox)
}
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(out, "{byte:02x}").expect("String writer");
    }
    out
}
fn stream(doc: &Document, id: ObjectId, limit: usize) -> Checked<&Stream> {
    let stream = pdf(obj(doc, id)?.as_stream(), "expected source stream object")?;
    bound(stream.content.len() <= limit, "source stream byte limit")?;
    require(
        !stream.dict.has(b"Filter")
            && !stream.dict.has(b"DecodeParms")
            && !stream.dict.has(b"UseCMap"),
        "filtered or inherited source stream unsupported",
    )?;
    Ok(stream)
}

pub fn verify_source(
    bytes: &[u8],
    limits: &SourceLimits,
    cancellation: &AtomicBool,
) -> VerifiedSource {
    let source = SourceIdentity::of(bytes);
    match inspect(bytes, limits, cancellation) {
        Ok(page) => VerifiedSource {
            source,
            outcome: SourceOutcome::Supported,
            pages: vec![page],
            issues: vec![],
        },
        Err(failure) => VerifiedSource {
            source,
            outcome: failure.outcome,
            pages: vec![],
            issues: vec![failure.issue],
        },
    }
}

fn inspect(bytes: &[u8], limits: &SourceLimits, cancellation: &AtomicBool) -> Checked<SourcePage> {
    cancelled(cancellation)?;
    bound(
        bytes.len() as u64 <= limits.max_source_bytes,
        "source byte limit",
    )?;
    // Loading uses lopdf's parser; object-count bounds are checked afterward.
    // This is not a parser RSS guarantee and callers must bound process resources.
    let document = pdf(Document::load_mem(bytes), "PDF could not be loaded")?;
    cancelled(cancellation)?;
    bound(
        document.objects.len() <= limits.max_objects,
        "PDF object count limit",
    )?;
    require(!document.is_encrypted(), "encrypted source unsupported")?;
    require(
        !document.trailer.has(b"Prev"),
        "incremental PDF revisions unsupported",
    )?;
    let catalog = pdf(document.catalog(), "catalog missing")?;
    for forbidden in [b"OCProperties".as_slice(), b"AcroForm", b"Perms"] {
        require(
            !catalog.has(forbidden),
            "optional content/forms unsupported",
        )?;
    }
    let pages = document.get_pages();
    require(pages.len() == 1, "profile requires exactly one page")?;
    let page_id = *pages
        .get(&1)
        .ok_or_else(|| unsupported("page one missing"))?;
    let page = dict(&document, page_id)?;
    let media = rectangle(key(page, b"MediaBox")?)?;
    let crop = rectangle(key(page, b"CropBox")?)?;
    require(
        media == crop && media.x0 == 0.0 && media.y0 == 0.0,
        "nonzero crop/media transform unsupported",
    )?;
    require(
        number(key(page, b"Rotate")?)? == 0.0,
        "rotated page unsupported",
    )?;
    for forbidden in [b"UserUnit".as_slice(), b"Annots", b"Group", b"VP", b"AF"] {
        require(
            !page.has(forbidden),
            "page transform/annotation/group unsupported",
        )?;
    }
    let resources = match key(page, b"Resources")? {
        Object::Dictionary(dict) => dict,
        Object::Reference(id) => dict(&document, *id)?,
        _ => return Err(unsupported("resources dictionary required")),
    };
    for forbidden in [
        b"XObject".as_slice(),
        b"ExtGState",
        b"Properties",
        b"Pattern",
        b"Shading",
    ] {
        require(
            !resources.has(forbidden),
            "unsupported resource graphics state",
        )?;
    }
    let fonts = pdf(
        key(resources, b"Font")?.as_dict(),
        "direct font resource dictionary required",
    )?;
    let content_id = reference(page, b"Contents")?;
    let content = stream(&document, content_id, limits.max_content_bytes)?;
    let operations = pdf(
        Content::decode_strict(&content.content),
        "content syntax not fully decoded",
    )?
    .operations;
    bound(
        operations.len() <= limits.max_operators,
        "content operator limit",
    )?;
    require(
        !operations.is_empty() && operations.len() % 5 == 0,
        "profile requires exact BT/Tf/Tm/Tj/ET blocks",
    )?;
    bound(
        operations.len() / 5 <= limits.max_text_shows,
        "text show count limit",
    )?;
    let mut witnesses = Vec::new();
    let mut used_bytes = 0_usize;
    let mut font_cmap_work = 0_usize;
    let mut font_bytes_work = 0_usize;
    for (block_index, block) in operations.as_chunks::<5>().0.iter().enumerate() {
        cancelled(cancellation)?;
        let ordinal = u32::try_from(block_index * 5 + 3)
            .map_err(|_| unsupported("operator ordinal overflow"))?;
        let checked: Checked<SourceTextWitness> = (|| {
            require(
                block
                    .iter()
                    .zip(["BT", "Tf", "Tm", "Tj", "ET"])
                    .all(|(op, expected)| op.operator == expected),
                "unknown operator/text state in executed block",
            )?;
            require(
                block[0].operands.is_empty()
                    && block[4].operands.is_empty()
                    && block[1].operands.len() == 2
                    && block[2].operands.len() == 6
                    && block[3].operands.len() == 1,
                "unexpected operator operands",
            )?;
            let resource = name(&block[1].operands[0])?;
            require(
                resource.is_ascii() && !resource.is_empty(),
                "non-ASCII/empty font resource name unsupported",
            )?;
            let size = number(&block[1].operands[1])?;
            require(size > 0.0, "nonpositive font size")?;
            let mut matrix = [0.0; 6];
            for (value, operand) in matrix.iter_mut().zip(&block[2].operands) {
                *value = number(operand)?;
            }
            require(
                matrix[0] > 0.0 && matrix[3] > 0.0 && matrix[1] == 0.0 && matrix[2] == 0.0,
                "non-axis-aligned text matrix unsupported",
            )?;
            let raw = pdf(block[3].operands[0].as_str(), "Tj string required")?;
            used_bytes = used_bytes
                .checked_add(raw.len())
                .ok_or_else(|| unsupported("shown byte count overflow"))?;
            bound(used_bytes <= limits.max_text_bytes, "shown text byte limit")?;
            require(
                !raw.is_empty() && raw.len() % 2 == 0,
                "fixed two-byte shown codes required",
            )?;
            let font_id = pdf(
                key(fonts, resource)?.as_reference(),
                "indirect font resource required",
            )?;
            let font = inspect_font(
                &document,
                font_id,
                raw,
                limits,
                cancellation,
                &mut font_cmap_work,
                &mut font_bytes_work,
            )?;
            let mut region: Option<SourceBox> = None;
            let mut advance = 0.0;
            for width in &font.widths {
                let b = font.bbox;
                let box_for_glyph = SourceBox {
                    x0: matrix[4] + matrix[0] * (advance + size * b.x0 / 1000.0),
                    x1: matrix[4] + matrix[0] * (advance + size * b.x1 / 1000.0),
                    y0: matrix[5] + matrix[3] * size * b.y0 / 1000.0,
                    y1: matrix[5] + matrix[3] * size * b.y1 / 1000.0,
                };
                region = Some(match region {
                    None => box_for_glyph,
                    Some(previous) => SourceBox {
                        x0: previous.x0.min(box_for_glyph.x0),
                        y0: previous.y0.min(box_for_glyph.y0),
                        x1: previous.x1.max(box_for_glyph.x1),
                        y1: previous.y1.max(box_for_glyph.y1),
                    },
                });
                advance += size * width / 1000.0;
            }
            let region = region.ok_or_else(|| unsupported("empty geometry"))?;
            require(
                [region.x0, region.x1, region.y0, region.y1]
                    .into_iter()
                    .all(f64::is_finite)
                    && region.x0 >= 0.0
                    && region.y0 >= 0.0
                    && region.x1 <= media.x1
                    && region.y1 <= media.y1,
                "computed source region outside page",
            )?;
            Ok(SourceTextWitness {
                id: format!("p1/object{}-{}/op{ordinal}", content_id.0, content_id.1),
                region,
                unicode: font.unicode,
                origin: SourceOrigin {
                    page_object: page_id,
                    content_object: content_id,
                    content_sha256: Digest::of(&content.content),
                    operator_ordinal: ordinal,
                    raw_codes_hex: hex(raw),
                    font_resource: String::from_utf8_lossy(resource).into_owned(),
                    font_object: font_id,
                    descendant_font_object: font.descendant,
                    encoding_object: font.encoding_id,
                    encoding_sha256: font.encoding_hash,
                    to_unicode_object: font.unicode_id,
                    to_unicode_sha256: font.unicode_hash,
                    descriptor_object: font.descriptor,
                    font_program_object: font.program,
                    font_program_sha256: font.program_hash,
                    text_matrix: matrix,
                    font_size: size,
                    explicit_widths: font.widths,
                    declared_font_bbox: font.declared_bbox,
                    embedded_font_global_bbox: font.embedded_bbox,
                    geometry_font_bbox: font.bbox,
                    character_codes: font.codes.clone(),
                    glyph_ids: font.codes,
                    unicode_scalars: font.scalars,
                },
            })
        })();
        match checked {
            Ok(witness) => witnesses.push(witness),
            Err(mut error) => {
                error.issue.operator_ordinal = Some(ordinal);
                error.issue.raw_codes_hex = block[3]
                    .operands
                    .first()
                    .and_then(|object| object.as_str().ok())
                    .map(hex);
                error.issue.font_resource = block[1]
                    .operands
                    .first()
                    .and_then(|object| object.as_name().ok())
                    .map(|name| String::from_utf8_lossy(name).into_owned());
                return Err(error);
            }
        }
    }
    for (index, left) in witnesses.iter().enumerate() {
        for right in witnesses.iter().skip(index + 1) {
            cancelled(cancellation)?;
            require(
                !(left.region.x0 < right.region.x1
                    && left.region.x1 > right.region.x0
                    && left.region.y0 < right.region.y1
                    && left.region.y1 > right.region.y0),
                "overlapping/duplicate source text regions unsupported",
            )?;
        }
    }
    Ok(SourcePage {
        page: 1,
        width: media.x1,
        height: media.y1,
        rotation: 0,
        witnesses,
    })
}

struct FontEvidence {
    descendant: ObjectId,
    encoding_id: Option<ObjectId>,
    encoding_hash: Digest,
    unicode_id: ObjectId,
    unicode_hash: Digest,
    descriptor: ObjectId,
    program: ObjectId,
    program_hash: Digest,
    bbox: SourceBox,
    declared_bbox: SourceBox,
    embedded_bbox: SourceBox,
    widths: Vec<f64>,
    codes: Vec<u16>,
    scalars: Vec<u32>,
    unicode: String,
}

fn inspect_font(
    doc: &Document,
    font_id: ObjectId,
    raw: &[u8],
    limits: &SourceLimits,
    cancellation: &AtomicBool,
    font_cmap_work: &mut usize,
    font_bytes_work: &mut usize,
) -> Checked<FontEvidence> {
    let font = dict(doc, font_id)?;
    require(
        name(key(font, b"Subtype")?)? == b"Type0",
        "only Type0 text fonts supported",
    )?;
    for field in [b"FontMatrix".as_slice(), b"UseCMap", b"WMode"] {
        require(!font.has(field), "unsupported font state")?;
    }
    let (encoding_id, encoding_hash) = match key(font, b"Encoding")? {
        Object::Name(value) if value == b"Identity-H" => (None, Digest::of(b"/Identity-H")),
        Object::Reference(id) => {
            let bytes = &stream(doc, *id, limits.max_cmap_bytes)?.content;
            parse_cmap(bytes, true, limits, cancellation)?;
            (Some(*id), Digest::of(bytes))
        }
        _ => {
            return Err(unsupported(
                "only fixed-width horizontal identity encoding supported",
            ));
        }
    };
    let unicode_id = reference(font, b"ToUnicode")?;
    let unicode_bytes = &stream(doc, unicode_id, limits.max_cmap_bytes)?.content;
    let mapping = parse_cmap(unicode_bytes, false, limits, cancellation)?;
    let descendants = pdf(
        key(font, b"DescendantFonts")?.as_array(),
        "descendant font list required",
    )?;
    require(
        descendants.len() == 1,
        "exactly one descendant font required",
    )?;
    let descendant = pdf(
        descendants[0].as_reference(),
        "indirect descendant font required",
    )?;
    let cid = dict(doc, descendant)?;
    require(
        name(key(cid, b"Subtype")?)? == b"CIDFontType2"
            && name(key(cid, b"CIDToGIDMap")?)? == b"Identity",
        "identity CID-to-GID TrueType descendant required",
    )?;
    for field in [b"W2".as_slice(), b"DW2", b"FontMatrix", b"UseCMap"] {
        require(!cid.has(field), "vertical or transformed font unsupported")?;
    }
    let widths = parse_widths(key(cid, b"W")?, limits)?;
    let descriptor = reference(cid, b"FontDescriptor")?;
    let descriptor_dict = dict(doc, descriptor)?;
    require(
        !descriptor_dict.has(b"FontFile") && !descriptor_dict.has(b"FontFile3"),
        "unsupported embedded font program type",
    )?;
    let declared = rectangle(key(descriptor_dict, b"FontBBox")?)?;
    let program = reference(descriptor_dict, b"FontFile2")?;
    let program_stream = pdf(
        obj(doc, program)?.as_stream(),
        "font program stream required",
    )?;
    bound(
        program_stream.content.len() <= limits.max_font_bytes,
        "encoded font byte limit",
    )?;
    *font_bytes_work = font_bytes_work
        .checked_add(program_stream.content.len())
        .ok_or_else(|| unsupported("font work byte overflow"))?;
    bound(
        *font_bytes_work <= limits.max_total_font_bytes,
        "cumulative font byte limit",
    )?;
    require(
        !program_stream.dict.has(b"DecodeParms"),
        "font DecodeParms unsupported",
    )?;
    let font_bytes = match program_stream.dict.get(b"Filter") {
        Err(_) => program_stream.content.clone(),
        Ok(Object::Name(filter)) if filter == b"FlateDecode" => {
            let mut bytes = Vec::new();
            flate2::read::ZlibDecoder::new(program_stream.content.as_slice())
                .take(limits.max_font_bytes.saturating_add(1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|_| unsupported("font decompression failed"))?;
            bound(
                bytes.len() <= limits.max_font_bytes,
                "decoded font byte limit",
            )?;
            bytes
        }
        _ => return Err(unsupported("unsupported font stream filter")),
    };
    *font_bytes_work = font_bytes_work
        .checked_add(font_bytes.len())
        .ok_or_else(|| unsupported("font work byte overflow"))?;
    bound(
        *font_bytes_work <= limits.max_total_font_bytes,
        "cumulative font byte limit",
    )?;
    cancelled(cancellation)?;
    let face = ttf_parser::Face::parse(&font_bytes, 0)
        .map_err(|_| unsupported("invalid embedded TrueType font"))?;
    require(
        font_bytes.get(..4) == Some(&[0, 1, 0, 0]) && !face.is_variable(),
        "font collections/variable/non-TrueType programs unsupported",
    )?;
    require(
        face.tables().glyf.is_some(),
        "TrueType glyph outlines required",
    )?;
    let units = f64::from(face.units_per_em());
    require(units > 0.0, "invalid font units per em")?;
    let actual = face.global_bounding_box();
    let actual = SourceBox {
        x0: f64::from(actual.x_min) * 1000.0 / units,
        y0: f64::from(actual.y_min) * 1000.0 / units,
        x1: f64::from(actual.x_max) * 1000.0 / units,
        y1: f64::from(actual.y_max) * 1000.0 / units,
    };
    // 0.001 em-normalized font unit tolerates PDF f32/decimal metric rounding.
    // Geometry uses the union, not the tolerance, so no glyph enclosure is lost.
    require(
        declared.x0 <= actual.x0 + 0.001
            && declared.y0 <= actual.y0 + 0.001
            && declared.x1 >= actual.x1 - 0.001
            && declared.y1 >= actual.y1 - 0.001,
        "declared FontBBox does not enclose embedded global bounds",
    )?;
    let bbox = SourceBox {
        x0: declared.x0.min(actual.x0),
        y0: declared.y0.min(actual.y0),
        x1: declared.x1.max(actual.x1),
        y1: declared.y1.max(actual.y1),
    };
    let shown: BTreeSet<u16> = raw
        .as_chunks::<2>()
        .0
        .iter()
        .map(|code| u16::from_be_bytes([code[0], code[1]]))
        .collect();
    let font_cmap = face
        .tables()
        .cmap
        .ok_or_else(|| unsupported("embedded Unicode cmap missing"))?;
    let mut reverse = BTreeMap::new();
    let mut unicode_tables = 0_usize;
    for table in font_cmap.subtables {
        if !table.is_unicode() {
            continue;
        }
        unicode_tables += 1;
        require(
            matches!(
                table.format,
                ttf_parser::cmap::Format::SegmentMappingToDeltaValues(_)
            ),
            "only bounded BMP format-4 Unicode font cmap supported",
        )?;
        for scalar in 0..=u32::from(u16::MAX) {
            cancelled(cancellation)?;
            bound(
                *font_cmap_work < limits.max_font_cmap_probes,
                "font cmap probe limit",
            )?;
            *font_cmap_work += 1;
            if let Some(glyph) = table.glyph_index(scalar)
                && shown.contains(&glyph.0)
                && let Some(previous) = reverse.insert(glyph.0, scalar)
            {
                require(
                    previous == scalar,
                    "ambiguous Unicode aliases for shown glyph",
                )?;
            }
        }
    }
    require(unicode_tables > 0, "embedded Unicode font cmap missing")?;
    let mut codes = Vec::new();
    let mut scalars = Vec::new();
    let mut used_widths = Vec::new();
    let mut unicode = String::new();
    for code in raw.as_chunks::<2>().0 {
        cancelled(cancellation)?;
        let code = u16::from_be_bytes([code[0], code[1]]);
        require(
            code > 0 && code < face.number_of_glyphs(),
            "missing/.notdef glyph unsupported",
        )?;
        let scalar = *mapping
            .get(&code)
            .ok_or_else(|| unsupported("incomplete ToUnicode mapping for shown code"))?;
        let mut matched = false;
        let cmap = face
            .tables()
            .cmap
            .ok_or_else(|| unsupported("embedded Unicode cmap missing"))?;
        for table in cmap.subtables {
            if table.is_unicode() {
                require(
                    table.glyph_index(u32::from(scalar)) == Some(ttf_parser::GlyphId(code)),
                    "ToUnicode conflicts with embedded Unicode-to-GID map",
                )?;
                matched = true;
            }
        }
        require(matched, "mapped Unicode lacks embedded font corroboration")?;
        let width = *widths
            .get(&code)
            .ok_or_else(|| unsupported("explicit width missing for shown code"))?;
        let font_width = f64::from(
            face.glyph_hor_advance(ttf_parser::GlyphId(code))
                .ok_or_else(|| unsupported("glyph advance missing"))?,
        ) * 1000.0
            / units;
        require(
            (width - font_width).abs() <= 0.001,
            "PDF width disagrees with embedded glyph advance",
        )?;
        if let Some(glyph_box) = face.glyph_bounding_box(ttf_parser::GlyphId(code)) {
            require(
                f64::from(glyph_box.x_min) * 1000.0 / units >= bbox.x0
                    && f64::from(glyph_box.x_max) * 1000.0 / units <= bbox.x1
                    && f64::from(glyph_box.y_min) * 1000.0 / units >= bbox.y0
                    && f64::from(glyph_box.y_max) * 1000.0 / units <= bbox.y1,
                "glyph bounds escape verified global font box",
            )?;
        }
        codes.push(code);
        scalars.push(u32::from(scalar));
        used_widths.push(width);
        unicode.push(scalar);
    }
    Ok(FontEvidence {
        descendant,
        encoding_id,
        encoding_hash,
        unicode_id,
        unicode_hash: Digest::of(unicode_bytes),
        descriptor,
        program,
        program_hash: Digest::of(&font_bytes),
        bbox,
        declared_bbox: declared,
        embedded_bbox: actual,
        widths: used_widths,
        codes,
        scalars,
        unicode,
    })
}

fn parse_widths(object: &Object, limits: &SourceLimits) -> Checked<BTreeMap<u16, f64>> {
    let array = pdf(object.as_array(), "explicit W array required")?;
    require(
        array.len() % 2 == 0,
        "only explicit CID [width...] W groups supported",
    )?;
    let mut widths = BTreeMap::new();
    for group in array.as_chunks::<2>().0 {
        let start = u16::try_from(pdf(group[0].as_i64(), "CID width start required")?)
            .map_err(|_| unsupported("invalid CID width start"))?;
        let values = pdf(group[1].as_array(), "explicit width group required")?;
        for (offset, value) in values.iter().enumerate() {
            bound(widths.len() < limits.max_cmap_entries, "width entry limit")?;
            let code = u16::try_from(usize::from(start) + offset)
                .map_err(|_| unsupported("CID width overflow"))?;
            let width = number(value)?;
            require(width > 0.0, "nonpositive explicit glyph width")?;
            require(
                widths.insert(code, width).is_none(),
                "duplicate explicit width mapping",
            )?;
        }
    }
    Ok(widths)
}

fn parse_cmap(
    bytes: &[u8],
    identity: bool,
    limits: &SourceLimits,
    cancellation: &AtomicBool,
) -> Checked<BTreeMap<u16, char>> {
    bound(bytes.len() <= limits.max_cmap_bytes, "CMap byte limit")?;
    let ops = pdf(
        Content::decode_strict(bytes),
        "CMap syntax not fully decoded",
    )?
    .operations;
    bound(ops.len() <= limits.max_operators, "CMap operator limit")?;
    let prefix = ["findresource", "begin", "dict", "begin", "begincmap"];
    let suffix = [
        "endcmap",
        "CMapName",
        "currentdict",
        "defineresource",
        "pop",
        "end",
        "end",
    ];
    require(
        ops.len() >= prefix.len() + suffix.len()
            && ops
                .iter()
                .take(prefix.len())
                .zip(prefix)
                .all(|(op, expected)| op.operator == expected)
            && ops
                .iter()
                .skip(ops.len() - suffix.len())
                .zip(suffix)
                .all(|(op, expected)| op.operator == expected),
        "CMap wrapper/trailer outside exact supported sequence",
    )?;
    let mut mapping = BTreeMap::new();
    let mut codespace = false;
    let mut cidrange = false;
    let mut kind = false;
    let mut horizontal = false;
    let mut metadata = BTreeSet::new();
    let mut cmap_started = false;
    let mut cmap_ended = false;
    let mut pending: Option<(&str, usize)> = None;
    for op in &ops {
        cancelled(cancellation)?;
        match op.operator.as_str() {
            "begincmap" => {
                require(
                    !cmap_started && !cmap_ended && op.operands.is_empty(),
                    "duplicate/bad CMap start",
                )?;
                cmap_started = true;
            }
            "endcmap" => {
                require(
                    cmap_started && !cmap_ended && pending.is_none() && op.operands.is_empty(),
                    "bad CMap end",
                )?;
                cmap_ended = true;
            }
            "def" => {
                require(
                    cmap_started && !cmap_ended && pending.is_none() && op.operands.len() == 2,
                    "bad CMap metadata",
                )?;
                let field = name(&op.operands[0])?;
                require(metadata.insert(field.to_vec()), "duplicate CMap metadata")?;
                match field {
                    b"CMapType" => {
                        require(
                            number(&op.operands[1])? == if identity { 1.0 } else { 2.0 },
                            "wrong CMap type",
                        )?;
                        kind = true;
                    }
                    b"WMode" => {
                        require(number(&op.operands[1])? == 0.0, "vertical CMap unsupported")?;
                        horizontal = true;
                    }
                    b"CMapName" => {
                        name(&op.operands[1])?;
                    }
                    b"CIDSystemInfo" => {
                        let info = pdf(op.operands[1].as_dict(), "invalid CIDSystemInfo")?;
                        require(
                            pdf(
                                key(info, b"Registry")?.as_str(),
                                "CID registry string required",
                            )? == b"Adobe"
                                && pdf(
                                    key(info, b"Ordering")?.as_str(),
                                    "CID ordering string required",
                                )? == if identity {
                                    b"Identity".as_slice()
                                } else {
                                    b"UCS".as_slice()
                                }
                                && number(key(info, b"Supplement")?)? == 0.0,
                            "unsupported CIDSystemInfo wrapper",
                        )?;
                    }
                    _ => return Err(unsupported("unknown CMap metadata/state")),
                }
            }
            "begincodespacerange" | "beginbfchar" | "begincidrange" => {
                require(
                    cmap_started && !cmap_ended && pending.is_none() && op.operands.len() == 1,
                    "bad CMap mapping block",
                )?;
                let count = usize::try_from(pdf(op.operands[0].as_i64(), "CMap count required")?)
                    .map_err(|_| unsupported("invalid CMap count"))?;
                bound(count <= limits.max_cmap_entries, "CMap entry limit")?;
                pending = Some((op.operator.as_str(), count));
            }
            "endcodespacerange" => {
                require(
                    pending == Some(("begincodespacerange", 1))
                        && !codespace
                        && op.operands.len() == 2,
                    "ambiguous code space",
                )?;
                require(
                    pdf(op.operands[0].as_str(), "code space string")? == [0, 0]
                        && pdf(op.operands[1].as_str(), "code space string")? == [255, 255],
                    "fixed full two-byte codespace required",
                )?;
                codespace = true;
                pending = None;
            }
            "endcidrange" => {
                require(
                    identity
                        && pending == Some(("begincidrange", 1))
                        && !cidrange
                        && op.operands.len() == 3,
                    "only one identity CID range supported",
                )?;
                require(
                    pdf(op.operands[0].as_str(), "CID code string")? == [0, 0]
                        && pdf(op.operands[1].as_str(), "CID code string")? == [255, 255]
                        && number(&op.operands[2])? == 0.0,
                    "nonidentity CID range",
                )?;
                cidrange = true;
                pending = None;
            }
            "endbfchar" => {
                let Some(("beginbfchar", count)) = pending else {
                    return Err(unsupported("bfchar block mismatch"));
                };
                require(
                    !identity && op.operands.len() == count * 2,
                    "bfchar entry count mismatch",
                )?;
                for pair in op.operands.as_chunks::<2>().0 {
                    bound(
                        mapping.len() < limits.max_cmap_entries,
                        "CMap total entry limit",
                    )?;
                    let code = pdf(pair[0].as_str(), "bfchar source string")?;
                    let target = pdf(pair[1].as_str(), "bfchar target string")?;
                    require(
                        code.len() == 2 && !target.is_empty() && target.len() % 2 == 0,
                        "invalid bfchar code width",
                    )?;
                    let utf16: Vec<_> = target
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
                        .collect();
                    let decoded = String::from_utf16(&utf16)
                        .map_err(|_| unsupported("invalid Unicode surrogate mapping"))?;
                    let mut chars = decoded.chars();
                    let scalar = chars
                        .next()
                        .ok_or_else(|| unsupported("empty bfchar Unicode"))?;
                    require(
                        chars.next().is_none() && !scalar.is_control(),
                        "only one noncontrol Unicode scalar per code supported",
                    )?;
                    require(
                        mapping
                            .insert(u16::from_be_bytes([code[0], code[1]]), scalar)
                            .is_none(),
                        "duplicate/ambiguous bfchar code",
                    )?;
                }
                pending = None;
            }
            // Restricted standard CMap wrapper: these carry no mapping/state.
            "findresource" => {
                require(
                    !cmap_started
                        && op.operands.len() == 2
                        && name(&op.operands[0])? == b"CIDInit"
                        && name(&op.operands[1])? == b"ProcSet",
                    "unsupported CMap resource operation",
                )?;
            }
            "dict" => {
                require(
                    !cmap_started && op.operands.len() == 1 && number(&op.operands[0])? == 12.0,
                    "unsupported CMap dictionary wrapper",
                )?;
            }
            "begin" => {
                require(
                    !cmap_started && op.operands.is_empty(),
                    "unsupported CMap begin scope",
                )?;
            }
            "CMapName" | "currentdict" | "pop" | "end" => {
                require(
                    cmap_ended && op.operands.is_empty(),
                    "unsupported CMap trailing operation",
                )?;
            }
            "defineresource" => {
                require(
                    cmap_ended && op.operands.len() == 1 && name(&op.operands[0])? == b"CMap",
                    "unsupported CMap resource definition",
                )?;
            }
            _ => return Err(unsupported("unsupported CMap operator/mapping syntax")),
        }
    }
    require(
        metadata.contains(b"CMapName".as_slice())
            && metadata.contains(b"CIDSystemInfo".as_slice())
            && cmap_started
            && cmap_ended
            && pending.is_none()
            && codespace
            && kind
            && horizontal
            && (!identity || cidrange),
        "incomplete CMap declaration",
    )?;
    Ok(mapping)
}
