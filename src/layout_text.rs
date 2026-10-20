//! Bounded physical text layout over immutable extraction evidence.
//!
//! This is an opt-in character grid, not semantic reading order. It never
//! consults cleaned `PageText::text` or `lines`, trims text, or mutates spans.
//! Only `std` and the three evidence types from `schema` cross this boundary;
//! PDF decoding and browser/WASI I/O belong to callers. See `docs/LAYOUT_TEXT.md`.

use std::fmt;
use std::ops::Range;

use crate::schema::{BBox, PageText, Span};

const HARD_COLUMNS: usize = 4096;
const HARD_ROWS: usize = 20_000;
const HARD_SPANS: usize = 100_000;
const HARD_BYTES: usize = 8 * 1024 * 1024;

/// Grid pitches in PDF points; `None` estimates from median span evidence.
/// Limits may be reduced by callers, but cannot exceed the hard ceilings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutOptions {
    pub column_pitch: Option<f32>,
    pub row_pitch: Option<f32>,
    pub max_columns: usize,
    pub max_rows: usize,
    pub max_spans: usize,
    pub max_output_bytes: usize,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            column_pitch: None,
            row_pitch: None,
            max_columns: 512,
            max_rows: 4096,
            max_spans: 20_000,
            max_output_bytes: 2 * 1024 * 1024,
        }
    }
}

/// Placement uncertainty; fallback text is preserved verbatim after the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutIssue {
    MissingGeometry,
    InvalidGeometry,
    OutsidePage,
    /// Tall boxes suggest vertical text, but spans carry no text direction.
    VerticalText,
    /// Control characters cannot be positioned as a single grid run.
    ControlText,
    /// A collision moved this run right, keeping all source characters.
    OverlapShift,
}

/// One entry per input span (including empty strings), in emitted order.
/// `bytes` selects exactly the original span text, excluding generated whitespace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    pub span_index: usize,
    pub bytes: Range<usize>,
    /// Zero-based actual output row and scalar column, or `None` for fallback.
    pub row: Option<usize>,
    pub column: Option<usize>,
    pub issue: Option<LayoutIssue>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutText {
    pub text: String,
    pub placements: Vec<Placement>,
    pub column_pitch: f64,
    pub row_pitch: f64,
}

/// Failure returns no partial output; evidence is never changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutError {
    InvalidOptions,
    InvalidPage,
    UnsupportedRotation,
    SpanLimit,
    ColumnLimit,
    RowLimit,
    OutputLimit,
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidOptions => "invalid layout pitches or limits",
            Self::InvalidPage => "layout requires finite positive page dimensions",
            Self::UnsupportedRotation => "layout requires a multiple of 90 degrees",
            Self::SpanLimit => "layout span limit exceeded",
            Self::ColumnLimit => "layout column limit exceeded",
            Self::RowLimit => "layout row limit exceeded",
            Self::OutputLimit => "layout output byte limit exceeded",
        })
    }
}

impl std::error::Error for LayoutError {}

fn positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn validate(options: LayoutOptions) -> Result<(), LayoutError> {
    if options.column_pitch.is_some_and(|p| !positive(p.into()))
        || options.row_pitch.is_some_and(|p| !positive(p.into()))
        || !(1..=HARD_COLUMNS).contains(&options.max_columns)
        || !(1..=HARD_ROWS).contains(&options.max_rows)
        || !(1..=HARD_SPANS).contains(&options.max_spans)
        || !(1..=HARD_BYTES).contains(&options.max_output_bytes)
    {
        return Err(LayoutError::InvalidOptions);
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Frame {
    width: f64,
    height: f64,
    rotation: i32,
}

impl Frame {
    fn point(self, x: f32, y: f32) -> (f64, f64) {
        let (x, y) = (f64::from(x), f64::from(y));
        match self.rotation {
            90 => (y, self.width - x),
            180 => (self.width - x, self.height - y),
            270 => (self.height - y, x),
            _ => (x, y),
        }
    }

    fn view_height(self) -> f64 {
        if matches!(self.rotation, 90 | 270) {
            self.width
        } else {
            self.height
        }
    }

    fn run(self, span: &Span, index: usize) -> Result<Run, LayoutIssue> {
        let b = span.bbox.ok_or(LayoutIssue::MissingGeometry)?;
        if !finite_box(b) || b.x1 < b.x0 || b.y1 < b.y0 {
            return Err(LayoutIssue::InvalidGeometry);
        }
        if b.x0 < 0.0 || b.y0 < 0.0 || f64::from(b.x1) > self.width || f64::from(b.y1) > self.height
        {
            return Err(LayoutIssue::OutsidePage);
        }
        if span.text.chars().any(char::is_control) {
            return Err(LayoutIssue::ControlText);
        }
        let a = self.point(b.x0, b.y0);
        let z = self.point(b.x1, b.y1);
        let width = (z.0 - a.0).abs();
        let height = (z.1 - a.1).abs();
        let chars = span.text.chars().count();
        let size = span
            .size
            .map(f64::from)
            .filter(|s| positive(*s))
            .unwrap_or(10.0);
        if chars > 1 && height > width * 2.0 && height > size * 2.0 {
            return Err(LayoutIssue::VerticalText);
        }
        Ok(Run {
            index,
            x: a.0.min(z.0),
            // Bottom edge is a baseline proxy, so mixed-size headings align.
            y: self.view_height() - a.1.min(z.1),
            width,
            height,
            chars,
        })
    }
}

fn finite_box(b: BBox) -> bool {
    [b.x0, b.y0, b.x1, b.y1].into_iter().all(f32::is_finite)
}

struct Run {
    index: usize,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    chars: usize,
}

fn median(mut values: Vec<f64>, fallback: f64) -> f64 {
    values.retain(|v| positive(*v));
    values.sort_unstable_by(f64::total_cmp);
    values.get(values.len() / 2).copied().unwrap_or(fallback)
}

struct Writer {
    text: String,
    row: usize,
    column: usize,
    options: LayoutOptions,
}

impl Writer {
    fn push(&mut self, text: &str) -> Result<Range<usize>, LayoutError> {
        let start = self.text.len();
        if text.len() > self.options.max_output_bytes - start {
            return Err(LayoutError::OutputLimit);
        }
        for ch in text.chars() {
            if ch == '\n' {
                self.row += 1;
                self.column = 0;
                if self.row >= self.options.max_rows {
                    return Err(LayoutError::RowLimit);
                }
            } else {
                self.column += 1;
                if self.column > self.options.max_columns {
                    return Err(LayoutError::ColumnLimit);
                }
            }
        }
        self.text.push_str(text);
        Ok(start..self.text.len())
    }

    fn spaces(&mut self, count: usize) -> Result<(), LayoutError> {
        if count > self.options.max_columns - self.column {
            return Err(LayoutError::ColumnLimit);
        }
        if count > self.options.max_output_bytes - self.text.len() {
            return Err(LayoutError::OutputLimit);
        }
        self.text.extend(std::iter::repeat_n(' ', count));
        self.column += count;
        Ok(())
    }

    fn place(
        &mut self,
        span: &Span,
        index: usize,
        positioned: bool,
        issue: Option<LayoutIssue>,
    ) -> Result<Placement, LayoutError> {
        let row = positioned.then_some(self.row);
        let column = positioned.then_some(self.column);
        let bytes = self.push(&span.text)?;
        Ok(Placement {
            span_index: index,
            bytes,
            row,
            column,
            issue,
        })
    }
}

/// Render original spans into physical rows, with a shared left origin at the
/// leftmost positioned span. No top/bottom page padding or terminal newline is
/// generated. Collisions shift right by one space; unusable geometry is appended
/// in `(seq, input index)` order after a blank line and reported per placement.
///
/// Pitches are median box width / Unicode scalar count and 1.2 × median box
/// height (fallback 5 and 12 points). Every Unicode scalar occupies one cell:
/// this is not a terminal display-width or glyph renderer. Grouping compares
/// sorted bottom edges to a fixed row anchor within 0.3 row pitches; it cannot
/// chain nearby rows together. Work is O(spans log spans + input/output bytes),
/// with no recursive or pairwise geometry search and no coordinate-sized canvas.
pub fn render_page(page: &PageText, options: LayoutOptions) -> Result<LayoutText, LayoutError> {
    validate(options)?;
    if !positive(page.width.into()) || !positive(page.height.into()) {
        return Err(LayoutError::InvalidPage);
    }
    let rotation = page.rotation.rem_euclid(360);
    if !matches!(rotation, 0 | 90 | 180 | 270) {
        return Err(LayoutError::UnsupportedRotation);
    }
    if page.spans.len() > options.max_spans {
        return Err(LayoutError::SpanLimit);
    }
    // Check bytes before inspecting characters, sorting, or allocating run lists.
    let mut input_bytes = 0usize;
    for span in &page.spans {
        input_bytes = input_bytes
            .checked_add(span.text.len())
            .ok_or(LayoutError::OutputLimit)?;
        if input_bytes > options.max_output_bytes {
            return Err(LayoutError::OutputLimit);
        }
    }
    let frame = Frame {
        width: page.width.into(),
        height: page.height.into(),
        rotation,
    };
    let mut runs = Vec::new();
    let mut loose = Vec::new();
    for (index, span) in page.spans.iter().enumerate() {
        if span.text.is_empty() {
            loose.push((index, None));
        } else {
            match frame.run(span, index) {
                Ok(run) => runs.push(run),
                Err(issue) => loose.push((index, Some(issue))),
            }
        }
    }
    let column_pitch = options.column_pitch.map_or_else(
        || median(runs.iter().map(|r| r.width / r.chars as f64).collect(), 5.0),
        f64::from,
    );
    let row_pitch = options.row_pitch.map_or_else(
        || 1.2 * median(runs.iter().map(|r| r.height).collect(), 10.0),
        f64::from,
    );
    let origin = runs
        .iter()
        .map(|r| r.x)
        .min_by(f64::total_cmp)
        .unwrap_or(0.0);
    runs.sort_unstable_by(|a, b| {
        a.y.total_cmp(&b.y)
            .then_with(|| a.x.total_cmp(&b.x))
            .then_with(|| page.spans[a.index].seq.cmp(&page.spans[b.index].seq))
            .then_with(|| a.index.cmp(&b.index))
    });
    let mut writer = Writer {
        text: String::new(),
        row: 0,
        column: 0,
        options,
    };
    let mut placements = Vec::with_capacity(page.spans.len());
    let top = runs.first().map_or(0.0, |r| r.y);
    let mut start = 0;
    while start < runs.len() {
        let anchor = runs[start].y;
        let end = start
            + runs[start..]
                .iter()
                .take_while(|r| r.y - anchor <= row_pitch * 0.3)
                .count();
        let projected = ((anchor - top) / row_pitch).round();
        if projected >= options.max_rows as f64 {
            return Err(LayoutError::RowLimit);
        }
        let row = if start == 0 {
            0
        } else {
            (projected as usize).max(writer.row + 1)
        };
        while writer.row < row {
            writer.push("\n")?;
        }
        runs[start..end].sort_unstable_by(|a, b| {
            a.x.total_cmp(&b.x)
                .then_with(|| page.spans[a.index].seq.cmp(&page.spans[b.index].seq))
                .then_with(|| a.index.cmp(&b.index))
        });
        for run in &runs[start..end] {
            let projected = ((run.x - origin) / column_pitch).round();
            if projected >= options.max_columns as f64 {
                return Err(LayoutError::ColumnLimit);
            }
            let target = projected as usize;
            let issue = if target < writer.column {
                writer.spaces(1)?;
                Some(LayoutIssue::OverlapShift)
            } else {
                writer.spaces(target - writer.column)?;
                None
            };
            placements.push(writer.place(&page.spans[run.index], run.index, true, issue)?);
        }
        start = end;
    }
    loose.sort_unstable_by_key(|(index, _)| (page.spans[*index].seq, *index));
    let mut fallback_started = false;
    for (index, issue) in loose {
        let span = &page.spans[index];
        if !span.text.is_empty() {
            if !writer.text.is_empty() {
                writer.push(if fallback_started { "\n" } else { "\n\n" })?;
            }
            fallback_started = true;
        }
        placements.push(writer.place(span, index, false, issue)?);
    }
    Ok(LayoutText {
        text: writer.text,
        placements,
        column_pitch,
        row_pitch,
    })
}
