//! Page selections such as `1-3,7,10-`.

use crate::error::{PdfOpsError, Result};

/// One inclusive range; `end == None` runs to the last page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Range {
    start: u32,
    end: Option<u32>,
}

/// A comma-separated list of 1-based pages and inclusive ranges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageSelection {
    ranges: Vec<Range>,
}

impl PageSelection {
    /// Parse `1-3,7,10-` (spaces allowed). Pages are 1-based; an open range
    /// (`10-`) runs to the end of the document.
    pub fn parse(spec: &str) -> Result<Self> {
        let mut ranges = Vec::new();
        for part in spec.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let range = if let Some((a, b)) = part.split_once('-') {
                let start = parse_page(a.trim(), spec)?;
                let end = if b.trim().is_empty() {
                    None
                } else {
                    Some(parse_page(b.trim(), spec)?)
                };
                if let Some(end) = end
                    && end < start
                {
                    return Err(PdfOpsError::Invalid(format!(
                        "page range `{part}` ends before it starts (in `{spec}`)"
                    )));
                }
                Range { start, end }
            } else {
                let page = parse_page(part, spec)?;
                Range {
                    start: page,
                    end: Some(page),
                }
            };
            ranges.push(range);
        }
        if ranges.is_empty() {
            return Err(PdfOpsError::Invalid(format!(
                "empty page selection `{spec}`"
            )));
        }
        Ok(Self { ranges })
    }

    /// The selected pages, in the order listed, each checked against
    /// `page_count`. Duplicates are kept (a page may be listed twice).
    pub fn resolve(&self, page_count: u32) -> Result<Vec<u32>> {
        let mut pages = Vec::new();
        for range in &self.ranges {
            let end = range.end.unwrap_or(page_count);
            if range.start > page_count || end > page_count {
                return Err(PdfOpsError::Invalid(format!(
                    "page selection reaches page {} but the document has {page_count} pages",
                    end.max(range.start)
                )));
            }
            pages.extend(range.start..=end);
        }
        Ok(pages)
    }

    /// True when `page` (1-based) is selected; open ranges match everything
    /// at or after their start.
    #[must_use]
    pub fn contains(&self, page: u32) -> bool {
        self.ranges
            .iter()
            .any(|r| page >= r.start && r.end.is_none_or(|end| page <= end))
    }
}

fn parse_page(text: &str, spec: &str) -> Result<u32> {
    match text.parse::<u32>() {
        Ok(n) if n >= 1 => Ok(n),
        _ => Err(PdfOpsError::Invalid(format!(
            "`{text}` is not a page number (pages are 1-based) in `{spec}`"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ranges_and_singles() {
        let sel = PageSelection::parse("1-3, 7,10-").unwrap();
        assert_eq!(sel.resolve(12).unwrap(), vec![1, 2, 3, 7, 10, 11, 12]);
        assert!(sel.contains(2));
        assert!(!sel.contains(4));
        assert!(sel.contains(99));
    }

    #[test]
    fn rejects_bad_specs() {
        assert!(PageSelection::parse("0").is_err());
        assert!(PageSelection::parse("3-1").is_err());
        assert!(PageSelection::parse("a").is_err());
        assert!(PageSelection::parse("").is_err());
        assert!(PageSelection::parse("5").unwrap().resolve(4).is_err());
    }
}
