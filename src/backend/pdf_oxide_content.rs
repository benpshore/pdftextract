//! Source-specific admission check for the pinned native text operator cap.
//!
//! This is a conservative adapter work ceiling, not an exact native operator
//! count or a fabricated native diagnostic. It cannot certify text completeness.

use pdf_oxide::PdfDocument;
use pdf_oxide::content::Operator;

use super::BackendError;

const MAX_TEXT_WORK: usize = 1_000_000;
const MAX_DECODED_BYTES: usize = 16 * 1024 * 1024;

fn exhausted() -> BackendError {
    BackendError::Limit(
        "operator preflight reached 1000000 conservative text work units; native text-region truncation is unsupported".into(),
    )
}

/// Check one decoded stream without retaining an operator vector. The caller
/// walking nested Forms must separately bound their aggregate decoded bytes.
pub(crate) fn reject_text_stream_operator_cutoff(bytes: &[u8]) -> Result<(), BackendError> {
    if bytes.len() > MAX_DECODED_BYTES {
        return Err(BackendError::Limit(
            "operator preflight decoded content exceeds 16 MiB".into(),
        ));
    }

    // In 0.3.78, parse_region_text_only increments op_count for an unbalanced
    // RestoreState even when its bracketed CTM path suppresses the callback.
    // Charge every raw Q byte up front, including those in strings/images.
    // This intentionally overcounts, but covers every suppressed restore
    // without introducing another lexical parser or accumulating operators.
    let mut work = 0_usize;
    for &byte in bytes {
        if byte == b'Q' {
            work += 1;
            if work >= MAX_TEXT_WORK {
                return Err(exhausted());
            }
        }
    }
    let mut cutoff = false;
    let parsed = pdf_oxide::content::parse_and_execute_text_only(bytes, |operator| {
        // Real restores were charged above. Synthetic balancing restores are
        // not counted by the native cap. Other injected context operators may
        // conservatively contribute to this adapter ceiling.
        if !matches!(operator, Operator::RestoreState) {
            work += 1;
        }
        if work >= MAX_TEXT_WORK {
            cutoff = true;
            Err(pdf_oxide::Error::Unsupported(
                "adapter conservative text work ceiling".into(),
            ))
        } else {
            Ok(())
        }
    });
    if cutoff {
        return Err(exhausted());
    }
    parsed.map_err(|error| BackendError::Malformed(format!("operator preflight: {error}")))
}

/// Decode the selected page and reject its known silent text-cap path before
/// extraction. Native decoding can allocate before this retained-byte check;
/// untrusted documents still require the supervised disposable worker.
pub(crate) fn reject_text_operator_cutoff(
    document: &PdfDocument,
    page: usize,
) -> Result<(), BackendError> {
    let content = document
        .get_page_content_data(page)
        .map_err(|error| BackendError::Page {
            page: u32::try_from(page + 1).unwrap_or(u32::MAX),
            message: format!("operator preflight content decoding: {error}"),
        })?;
    reject_text_stream_operator_cutoff(&content).map_err(|error| match error {
        BackendError::Malformed(message) => BackendError::Page {
            page: u32::try_from(page + 1).unwrap_or(u32::MAX),
            message,
        },
        other => other,
    })
}
