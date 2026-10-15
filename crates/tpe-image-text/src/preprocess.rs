//! Pure-Rust preprocessing before recognition: grayscale, percentile
//! auto-contrast, polarity (light-on-dark is inverted), a 2x upscale when the
//! estimated text height is small, and deskew by projection profile. Every
//! step is recorded in a [`PreprocessReport`] so the JSON output says exactly
//! what the engine saw. Inputs are never written; this works on a copy.

use image::imageops::FilterType;
use image::{DynamicImage, GrayImage, Luma, imageops};
use serde::{Deserialize, Serialize};

/// Text whose estimated height is below this many pixels is upscaled 2x.
pub const SMALL_TEXT_PX: u32 = 20;
/// Upscaling is skipped when the result would exceed this many pixels.
pub const MAX_UPSCALE_PIXELS: u64 = 40_000_000;
/// Deskew searches rotations in `[-MAX, MAX]` degrees.
pub const DESKEW_MAX_DEGREES: f32 = 5.0;
/// Deskew search step in degrees.
pub const DESKEW_STEP_DEGREES: f32 = 0.5;
/// Deskew search runs on a copy whose longest side is at most this.
const DESKEW_PROBE_SIDE: u32 = 800;
/// A deskew rotation is applied only when its profile score beats 0 degrees by this factor.
const DESKEW_MIN_GAIN: f32 = 1.05;

/// Which steps to run. All on by default.
#[derive(Clone, Copy, Debug)]
pub struct PreprocessOptions {
    /// Stretch the 1st..99th percentile of gray levels to 0..255.
    pub auto_contrast: bool,
    /// Upscale 2x when text looks smaller than [`SMALL_TEXT_PX`].
    pub upscale_small_text: bool,
    /// Rotate by the angle that maximises the row projection profile.
    pub deskew: bool,
}

impl Default for PreprocessOptions {
    fn default() -> Self {
        Self {
            auto_contrast: true,
            upscale_small_text: true,
            deskew: true,
        }
    }
}

/// What was done, with the measurements the decisions rested on.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PreprocessReport {
    /// Steps in the order applied.
    pub steps: Vec<String>,
    /// Width of the decoded input.
    pub input_width: u32,
    /// Height of the decoded input.
    pub input_height: u32,
    /// Width of the image handed to the engine.
    pub output_width: u32,
    /// Height of the image handed to the engine.
    pub output_height: u32,
    /// Gray level mapped to black by auto-contrast, when applied.
    pub contrast_low: Option<u8>,
    /// Gray level mapped to white by auto-contrast, when applied.
    pub contrast_high: Option<u8>,
    /// Otsu threshold used for binarisation.
    pub binarize_threshold: u8,
    /// The image was light-on-dark and was inverted.
    pub inverted: bool,
    /// Median height of ink rows, i.e. the text height estimate.
    pub estimated_text_height_px: Option<u32>,
    /// 1 when not upscaled, 2 when upscaled.
    pub upscale_factor: u32,
    /// Rotation applied in degrees (0 when none).
    pub deskew_degrees: f32,
}

/// Run the configured steps and describe them.
pub fn preprocess(image: &DynamicImage, opts: PreprocessOptions) -> (GrayImage, PreprocessReport) {
    let mut report = PreprocessReport {
        input_width: image.width(),
        input_height: image.height(),
        upscale_factor: 1,
        ..PreprocessReport::default()
    };
    let mut gray = image.to_luma8();
    report.steps.push("grayscale".to_string());

    if opts.auto_contrast
        && let Some((low, high)) = contrast_bounds(&gray)
    {
        stretch(&mut gray, low, high);
        report.contrast_low = Some(low);
        report.contrast_high = Some(high);
        report
            .steps
            .push(format!("auto-contrast {low}..{high} -> 0..255"));
    }

    let mut threshold = otsu_threshold(&histogram(&gray));
    if dark_fraction(&gray, threshold) > 0.5 {
        for p in gray.pixels_mut() {
            p.0[0] = 255 - p.0[0];
        }
        threshold = otsu_threshold(&histogram(&gray));
        report.inverted = true;
        report
            .steps
            .push("inverted (light text on dark background)".to_string());
    }
    report.binarize_threshold = threshold;

    let text_height = estimate_text_height(&gray, threshold);
    report.estimated_text_height_px = text_height;
    if opts.upscale_small_text
        && let Some(h) = text_height
        && h < SMALL_TEXT_PX
        && u64::from(gray.width()) * u64::from(gray.height()) * 4 <= MAX_UPSCALE_PIXELS
    {
        gray = imageops::resize(
            &gray,
            gray.width() * 2,
            gray.height() * 2,
            FilterType::CatmullRom,
        );
        report.upscale_factor = 2;
        report
            .steps
            .push(format!("upscale x2 (text height {h} px < {SMALL_TEXT_PX})"));
    }

    if opts.deskew
        && let Some(angle) = deskew_angle(&gray, threshold)
    {
        gray = rotate_gray(&gray, angle, 255);
        report.deskew_degrees = angle;
        report
            .steps
            .push(format!("deskew {angle:+.1} deg (projection profile)"));
    }

    report.output_width = gray.width();
    report.output_height = gray.height();
    (gray, report)
}

/// 256-bin histogram of gray levels.
pub fn histogram(gray: &GrayImage) -> [u64; 256] {
    let mut hist = [0u64; 256];
    for p in gray.pixels() {
        hist[usize::from(p.0[0])] += 1;
    }
    hist
}

/// 1st and 99th percentile gray levels when stretching them is worthwhile.
pub fn contrast_bounds(gray: &GrayImage) -> Option<(u8, u8)> {
    let hist = histogram(gray);
    let total: u64 = hist.iter().sum();
    if total == 0 {
        return None;
    }
    let low = percentile(&hist, total, 1);
    let high = percentile(&hist, total, 99);
    // Flat images gain nothing; images that already span the range are left alone.
    if high.saturating_sub(low) < 16 || (low < 8 && high > 247) {
        return None;
    }
    Some((low, high))
}

fn percentile(hist: &[u64; 256], total: u64, pct: u64) -> u8 {
    let target = (total * pct).div_ceil(100).max(1);
    let mut seen = 0u64;
    for (level, &count) in hist.iter().enumerate() {
        seen += count;
        if seen >= target {
            return u8::try_from(level).unwrap_or(255);
        }
    }
    255
}

fn stretch(gray: &mut GrayImage, low: u8, high: u8) {
    let span = f32::from(high) - f32::from(low);
    let lut: Vec<u8> = (0..=255u16)
        .map(|v| {
            let v = f32::from(v);
            ((v - f32::from(low)) * 255.0 / span)
                .round()
                .clamp(0.0, 255.0) as u8
        })
        .collect();
    for p in gray.pixels_mut() {
        p.0[0] = lut[usize::from(p.0[0])];
    }
}

/// Otsu's threshold: pixels strictly below it are ink.
pub fn otsu_threshold(hist: &[u64; 256]) -> u8 {
    let total: u64 = hist.iter().sum();
    if total == 0 {
        return 128;
    }
    let sum_all: f64 = hist
        .iter()
        .enumerate()
        .map(|(i, &c)| i as f64 * c as f64)
        .sum();
    let (mut below, mut sum_below, mut best, mut best_t) = (0.0f64, 0.0f64, -1.0f64, 128u8);
    for (t, &count) in hist.iter().enumerate() {
        below += count as f64;
        if below == 0.0 {
            continue;
        }
        let above = total as f64 - below;
        if above == 0.0 {
            break;
        }
        sum_below += t as f64 * count as f64;
        let mean_below = sum_below / below;
        let mean_above = (sum_all - sum_below) / above;
        let between = below * above * (mean_below - mean_above).powi(2);
        if between > best {
            best = between;
            best_t = u8::try_from(t + 1).unwrap_or(255);
        }
    }
    best_t
}

fn dark_fraction(gray: &GrayImage, threshold: u8) -> f32 {
    let dark = gray.pixels().filter(|p| p.0[0] < threshold).count();
    dark as f32 / gray.pixels().len().max(1) as f32
}

/// Ink count per row for pixels below `threshold`.
pub fn row_profile(gray: &GrayImage, threshold: u8) -> Vec<u32> {
    gray.rows()
        .map(|row| u32::try_from(row.filter(|p| p.0[0] < threshold).count()).unwrap_or(u32::MAX))
        .collect()
}

/// Median height of consecutive ink-bearing row runs; `None` without ink.
pub fn estimate_text_height(gray: &GrayImage, threshold: u8) -> Option<u32> {
    let min_ink = (gray.width() / 200).max(1);
    let mut runs: Vec<u32> = Vec::new();
    let mut current = 0u32;
    for ink in row_profile(gray, threshold) {
        if ink >= min_ink {
            current += 1;
        } else if current > 0 {
            runs.push(current);
            current = 0;
        }
    }
    if current > 0 {
        runs.push(current);
    }
    runs.retain(|&h| h >= 3);
    if runs.is_empty() {
        return None;
    }
    runs.sort_unstable();
    Some(runs[runs.len() / 2])
}

/// Rotate around the centre with bilinear sampling; uncovered pixels take `fill`.
pub fn rotate_gray(img: &GrayImage, degrees: f32, fill: u8) -> GrayImage {
    let (w, h) = img.dimensions();
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let (sin, cos) = degrees.to_radians().sin_cos();
    let mut out = GrayImage::from_pixel(w, h, Luma([fill]));
    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let sx = cos * dx + sin * dy + cx - 0.5;
            let sy = -sin * dx + cos * dy + cy - 0.5;
            out.put_pixel(x, y, Luma([bilinear(img, sx, sy, fill)]));
        }
    }
    out
}

fn bilinear(img: &GrayImage, x: f32, y: f32, fill: u8) -> u8 {
    let (w, h) = img.dimensions();
    if x < -1.0 || y < -1.0 || x > w as f32 || y > h as f32 {
        return fill;
    }
    let x0 = x.floor();
    let y0 = y.floor();
    let fx = x - x0;
    let fy = y - y0;
    let sample = |xi: f32, yi: f32| -> f32 {
        if xi < 0.0 || yi < 0.0 || xi >= w as f32 || yi >= h as f32 {
            f32::from(fill)
        } else {
            f32::from(img.get_pixel(xi as u32, yi as u32).0[0])
        }
    };
    let top = sample(x0, y0) * (1.0 - fx) + sample(x0 + 1.0, y0) * fx;
    let bottom = sample(x0, y0 + 1.0) * (1.0 - fx) + sample(x0 + 1.0, y0 + 1.0) * fx;
    (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8
}

/// Sum of squared row ink counts: high when text rows are horizontal.
pub fn profile_score(gray: &GrayImage, threshold: u8) -> f64 {
    row_profile(gray, threshold)
        .iter()
        .map(|&v| f64::from(v) * f64::from(v))
        .sum()
}

/// The rotation in `[-5, 5]` degrees (0.5 steps) that best aligns the rows,
/// or `None` when no rotation clearly beats the original.
pub fn deskew_angle(gray: &GrayImage, threshold: u8) -> Option<f32> {
    let longest = gray.width().max(gray.height());
    let probe = if longest > DESKEW_PROBE_SIDE {
        let scale = DESKEW_PROBE_SIDE as f32 / longest as f32;
        let w = ((gray.width() as f32 * scale).round() as u32).max(1);
        let h = ((gray.height() as f32 * scale).round() as u32).max(1);
        imageops::resize(gray, w, h, FilterType::Triangle)
    } else {
        gray.clone()
    };
    // Binarise once; nearest-neighbour rotation of 0/255 keeps the threshold valid.
    let mut binary = probe;
    for p in binary.pixels_mut() {
        p.0[0] = if p.0[0] < threshold { 0 } else { 255 };
    }
    let base = profile_score(&binary, 128);
    if base == 0.0 {
        return None;
    }
    let steps = (DESKEW_MAX_DEGREES / DESKEW_STEP_DEGREES).round() as i32;
    let mut best = (0.0f32, base);
    for i in -steps..=steps {
        if i == 0 {
            continue;
        }
        let angle = i as f32 * DESKEW_STEP_DEGREES;
        let rotated = rotate_nearest(&binary, angle);
        let score = profile_score(&rotated, 128);
        if score > best.1 {
            best = (angle, score);
        }
    }
    (best.0 != 0.0 && best.1 >= base * f64::from(DESKEW_MIN_GAIN)).then_some(best.0)
}

fn rotate_nearest(img: &GrayImage, degrees: f32) -> GrayImage {
    let (w, h) = img.dimensions();
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let (sin, cos) = degrees.to_radians().sin_cos();
    let mut out = GrayImage::from_pixel(w, h, Luma([255]));
    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let sx = (cos * dx + sin * dy + cx).floor();
            let sy = (-sin * dx + cos * dy + cy).floor();
            if sx >= 0.0 && sy >= 0.0 && sx < w as f32 && sy < h as f32 {
                out.put_pixel(x, y, *img.get_pixel(sx as u32, sy as u32));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_like(width: u32, height: u32, line_height: u32, gap: u32) -> GrayImage {
        let mut img = GrayImage::from_pixel(width, height, Luma([255]));
        let mut y = gap;
        while y + line_height < height {
            for yy in y..y + line_height {
                for x in (10..width - 10).step_by(3) {
                    img.put_pixel(x, yy, Luma([0]));
                }
            }
            y += line_height + gap;
        }
        img
    }

    #[test]
    fn otsu_separates_two_modes() {
        let mut hist = [0u64; 256];
        hist[30] = 1000;
        hist[220] = 1000;
        let t = otsu_threshold(&hist);
        assert!(t > 30 && t <= 220, "threshold {t}");
    }

    #[test]
    fn contrast_bounds_skip_full_range_and_flat_images() {
        let flat = GrayImage::from_pixel(10, 10, Luma([100]));
        assert_eq!(contrast_bounds(&flat), None);
        let mut full = GrayImage::from_pixel(100, 1, Luma([0]));
        for x in 0..100 {
            full.put_pixel(x, 0, Luma([(x * 255 / 99) as u8]));
        }
        assert_eq!(contrast_bounds(&full), None);
    }

    #[test]
    fn low_contrast_is_stretched_to_full_range() {
        let mut img = GrayImage::from_pixel(200, 20, Luma([120]));
        for x in 0..100 {
            for y in 0..20 {
                img.put_pixel(x, y, Luma([90]));
            }
        }
        let (out, report) =
            preprocess(&DynamicImage::ImageLuma8(img), PreprocessOptions::default());
        assert_eq!(report.contrast_low, Some(90));
        assert_eq!(report.contrast_high, Some(120));
        let hist = histogram(&out);
        assert!(
            hist[0] > 0 && hist[255] > 0,
            "stretched to both ends: {hist:?}"
        );
    }

    #[test]
    fn light_on_dark_is_inverted() {
        let mut img = GrayImage::from_pixel(100, 40, Luma([0]));
        for x in 10..90 {
            for y in 15..25 {
                img.put_pixel(x, y, Luma([255]));
            }
        }
        let opts = PreprocessOptions {
            upscale_small_text: false,
            deskew: false,
            ..PreprocessOptions::default()
        };
        let (out, report) = preprocess(&DynamicImage::ImageLuma8(img), opts);
        assert!(report.inverted);
        assert_eq!(out.get_pixel(0, 0).0[0], 255);
        assert_eq!(out.get_pixel(50, 20).0[0], 0);
    }

    #[test]
    fn text_height_is_the_median_run() {
        let img = text_like(300, 120, 12, 10);
        assert_eq!(estimate_text_height(&img, 128), Some(12));
        let blank = GrayImage::from_pixel(50, 50, Luma([255]));
        assert_eq!(estimate_text_height(&blank, 128), None);
    }

    #[test]
    fn small_text_is_upscaled_and_large_text_is_not() {
        let small = text_like(300, 120, 8, 10);
        let (out, report) = preprocess(
            &DynamicImage::ImageLuma8(small),
            PreprocessOptions::default(),
        );
        assert_eq!(report.upscale_factor, 2);
        assert_eq!((out.width(), out.height()), (600, 240));
        let big = text_like(300, 200, 30, 20);
        let (_, report) = preprocess(&DynamicImage::ImageLuma8(big), PreprocessOptions::default());
        assert_eq!(report.upscale_factor, 1);
        let (_, report) = preprocess(
            &DynamicImage::ImageLuma8(text_like(300, 120, 8, 10)),
            PreprocessOptions {
                upscale_small_text: false,
                ..PreprocessOptions::default()
            },
        );
        assert_eq!(report.upscale_factor, 1);
    }

    #[test]
    fn deskew_recovers_a_known_rotation() {
        let straight = text_like(400, 200, 10, 14);
        assert_eq!(
            deskew_angle(&straight, 128),
            None,
            "straight text needs no rotation"
        );
        let skewed = rotate_gray(&straight, 3.0, 255);
        let angle = deskew_angle(&skewed, 128).expect("skew detected");
        assert!((angle + 3.0).abs() <= 0.51, "angle {angle}");
        let fixed = rotate_gray(&skewed, angle, 255);
        assert!(profile_score(&fixed, 128) > profile_score(&skewed, 128) * 1.5);
    }

    #[test]
    fn rotation_by_zero_is_identity() {
        let img = text_like(60, 40, 5, 5);
        assert_eq!(rotate_gray(&img, 0.0, 255), img);
    }
}
