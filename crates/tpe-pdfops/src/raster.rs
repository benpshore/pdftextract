//! Raster clean-ups on 8-bit grey page images: adaptive background
//! normalisation (water stains, uneven lighting), projection-profile deskew
//! and simple baseline dewarp. Pure functions over `image::GrayImage`; no
//! renderer is involved, so they are testable without `PDFium`.
//!
//! Coordinates are image coordinates (x to the right, y down). A positive
//! rotation angle turns the picture clockwise on screen.

// Pixel arithmetic converts between float and integer coordinates throughout;
// every conversion is bounded by the image size and clamped where it matters.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap
)]

use image::{GrayImage, Luma};

/// Otsu's threshold: grey levels at or below it count as ink.
#[must_use]
pub fn otsu_threshold(img: &GrayImage) -> u8 {
    let mut hist = [0u64; 256];
    for p in img.pixels() {
        hist[p.0[0] as usize] += 1;
    }
    let total = u64::from(img.width()) * u64::from(img.height());
    if total == 0 {
        return 127;
    }
    let sum: f64 = hist
        .iter()
        .enumerate()
        .map(|(i, &c)| i as f64 * c as f64)
        .sum();
    let (mut sum_b, mut weight_b, mut best, mut best_t) = (0.0_f64, 0u64, 0.0_f64, 127u8);
    for (t, &count) in hist.iter().enumerate() {
        weight_b += count;
        if weight_b == 0 {
            continue;
        }
        let weight_f = total - weight_b;
        if weight_f == 0 {
            break;
        }
        sum_b += t as f64 * count as f64;
        let mean_b = sum_b / weight_b as f64;
        let mean_f = (sum - sum_b) / weight_f as f64;
        let between = weight_b as f64 * weight_f as f64 * (mean_b - mean_f).powi(2);
        if between > best {
            best = between;
            best_t = t as u8;
        }
    }
    best_t
}

/// Rotate the picture by `degrees` clockwise about its centre with bilinear
/// sampling; uncovered corners are white. The output has the input's size.
#[must_use]
pub fn rotate_gray(img: &GrayImage, degrees: f32) -> GrayImage {
    let (w, h) = img.dimensions();
    let (cx, cy) = ((w as f32 - 1.0) / 2.0, (h as f32 - 1.0) / 2.0);
    let (sin, cos) = degrees.to_radians().sin_cos();
    let mut out = GrayImage::new(w, h);
    for v in 0..h {
        for u in 0..w {
            let (du, dv) = (u as f32 - cx, v as f32 - cy);
            let x = cx + du * cos + dv * sin;
            let y = cy - du * sin + dv * cos;
            out.put_pixel(u, v, Luma([sample_bilinear(img, x, y)]));
        }
    }
    out
}

/// Bilinear sample with white outside the image.
fn sample_bilinear(img: &GrayImage, x: f32, y: f32) -> u8 {
    let (w, h) = (i64::from(img.width()), i64::from(img.height()));
    let (x0, y0) = (x.floor() as i64, y.floor() as i64);
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let at = |xi: i64, yi: i64| -> f32 {
        if xi < 0 || yi < 0 || xi >= w || yi >= h {
            255.0
        } else {
            f32::from(img.get_pixel(xi as u32, yi as u32).0[0])
        }
    };
    let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1, y0) * fx;
    let bottom = at(x0, y0 + 1) * (1.0 - fx) + at(x0 + 1, y0 + 1) * fx;
    (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8
}

/// Estimate the clockwise rotation (degrees, within `±max_degrees`) that
/// makes the text lines horizontal, by maximising the sharpness of the row
/// projection profile of the ink pixels. Returns 0 for an empty page.
#[must_use]
pub fn estimate_skew(img: &GrayImage, max_degrees: f32) -> f32 {
    let threshold = otsu_threshold(img);
    let (w, h) = img.dimensions();
    let (cx, cy) = ((w as f32 - 1.0) / 2.0, (h as f32 - 1.0) / 2.0);
    let ink: Vec<(f32, f32)> = img
        .enumerate_pixels()
        .filter(|(_, _, p)| p.0[0] <= threshold)
        .map(|(x, y, _)| (x as f32 - cx, y as f32 - cy))
        .collect();
    if ink.is_empty() || ink.len() == (w as usize * h as usize) {
        return 0.0;
    }
    let max = max_degrees.abs().max(0.1);
    let score = |degrees: f32| -> f64 {
        let (sin, cos) = degrees.to_radians().sin_cos();
        let rows = h as usize + 2 * (w as usize);
        let mut hist = vec![0u32; rows];
        for &(dx, dy) in &ink {
            let row = (cy + dx * sin + dy * cos).round() as i64 + i64::from(w);
            if row >= 0 && (row as usize) < rows {
                hist[row as usize] += 1;
            }
        }
        hist.iter().map(|&c| f64::from(c) * f64::from(c)).sum()
    };
    let search = |centre: f32, half: f32, step: f32| -> f32 {
        let steps = (2.0 * half / step).round() as i32;
        let mut best = (centre, f64::MIN);
        for i in 0..=steps {
            let angle = (centre - half + i as f32 * step).clamp(-max, max);
            let s = score(angle);
            // Ties (common on clean scans, where sub-pixel tilts round away)
            // go to the angle nearest zero.
            match s.total_cmp(&best.1) {
                std::cmp::Ordering::Greater => best = (angle, s),
                std::cmp::Ordering::Equal if angle.abs() < best.0.abs() => best = (angle, s),
                _ => {}
            }
        }
        best.0
    };
    let coarse = search(0.0, max, 0.5);
    let fine = search(coarse, 0.5, 0.1);
    search(fine, 0.1, 0.02)
}

/// Deskew: estimate the angle and rotate. Returns the image and the angle
/// applied (clockwise degrees).
#[must_use]
pub fn deskew(img: &GrayImage, max_degrees: f32) -> (GrayImage, f32) {
    let angle = estimate_skew(img, max_degrees);
    if angle.abs() < 0.01 {
        return (img.clone(), 0.0);
    }
    (rotate_gray(img, angle), angle)
}

/// Floor for the background estimate: a tile darker than this is treated as
/// content (a photograph, a solid fill), not as a stained background.
const BACKGROUND_FLOOR: f32 = 64.0;

/// Adaptive background normalisation: estimate the local background as the
/// 90th percentile of each `tile`x`tile` block, interpolate it bilinearly
/// and divide every pixel by it. Stains and lighting gradients become white
/// while ink keeps its contrast.
#[must_use]
pub fn normalise_background(img: &GrayImage, tile: u32) -> GrayImage {
    let tile = tile.max(4);
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return img.clone();
    }
    let tiles_x = w.div_ceil(tile) as usize;
    let tiles_y = h.div_ceil(tile) as usize;
    let mut background = vec![255.0_f32; tiles_x * tiles_y];
    for ty in 0..tiles_y {
        for tx in 0..tiles_x {
            let mut hist = [0u32; 256];
            let mut count = 0u32;
            for y in (ty as u32 * tile)..((ty as u32 + 1) * tile).min(h) {
                for x in (tx as u32 * tile)..((tx as u32 + 1) * tile).min(w) {
                    hist[img.get_pixel(x, y).0[0] as usize] += 1;
                    count += 1;
                }
            }
            let target = (f64::from(count) * 0.9).ceil() as u32;
            let mut seen = 0u32;
            let mut level = 255u8;
            for (value, &c) in hist.iter().enumerate() {
                seen += c;
                if seen >= target {
                    level = value as u8;
                    break;
                }
            }
            background[ty * tiles_x + tx] = f32::from(level).max(BACKGROUND_FLOOR);
        }
    }
    let bg_at = |x: u32, y: u32| -> f32 {
        let fx = ((x as f32 + 0.5) / tile as f32 - 0.5).max(0.0);
        let fy = ((y as f32 + 0.5) / tile as f32 - 0.5).max(0.0);
        let x0 = (fx.floor() as usize).min(tiles_x - 1);
        let y0 = (fy.floor() as usize).min(tiles_y - 1);
        let x1 = (x0 + 1).min(tiles_x - 1);
        let y1 = (y0 + 1).min(tiles_y - 1);
        let (ax, ay) = (fx - x0 as f32, fy - y0 as f32);
        let top = background[y0 * tiles_x + x0] * (1.0 - ax) + background[y0 * tiles_x + x1] * ax;
        let bottom =
            background[y1 * tiles_x + x0] * (1.0 - ax) + background[y1 * tiles_x + x1] * ax;
        top * (1.0 - ay) + bottom * ay
    };
    let mut out = GrayImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let bg = bg_at(x, y).max(1.0);
            let value = f32::from(img.get_pixel(x, y).0[0]) * 255.0 / bg;
            out.put_pixel(x, y, Luma([value.round().clamp(0.0, 255.0) as u8]));
        }
    }
    out
}

/// Rows of the text-line centres in each of `strips` vertical strips, found
/// as peaks of the strip's ink projection profile.
#[must_use]
pub fn line_peaks(img: &GrayImage, strips: u32) -> Vec<Vec<f32>> {
    let strips = strips.max(1);
    let threshold = otsu_threshold(img);
    let (w, h) = img.dimensions();
    let strip_w = w.div_ceil(strips).max(1);
    let mut result = Vec::with_capacity(strips as usize);
    for s in 0..strips {
        let x_start = s * strip_w;
        let x_end = ((s + 1) * strip_w).min(w);
        let mut profile = vec![0.0_f32; h as usize];
        if x_start < x_end {
            for y in 0..h {
                let mut ink = 0u32;
                for x in x_start..x_end {
                    if img.get_pixel(x, y).0[0] <= threshold {
                        ink += 1;
                    }
                }
                profile[y as usize] = ink as f32;
            }
        }
        result.push(peaks(&profile));
    }
    result
}

/// Local maxima of a smoothed profile that rise above a quarter of its peak,
/// each reported as the ink-weighted centre of its run.
fn peaks(profile: &[f32]) -> Vec<f32> {
    let n = profile.len();
    if n == 0 {
        return Vec::new();
    }
    let smooth: Vec<f32> = (0..n)
        .map(|i| {
            let lo = i.saturating_sub(1);
            let hi = (i + 1).min(n - 1);
            profile[lo..=hi].iter().sum::<f32>() / (hi - lo + 1) as f32
        })
        .collect();
    let max = smooth.iter().copied().fold(0.0_f32, f32::max);
    if max <= 0.0 {
        return Vec::new();
    }
    let floor = max * 0.25;
    let mut found = Vec::new();
    let mut i = 0;
    while i < n {
        if smooth[i] > floor {
            let start = i;
            while i < n && smooth[i] > floor {
                i += 1;
            }
            let run = &smooth[start..i];
            let weight: f32 = run.iter().sum();
            let centre: f32 = run
                .iter()
                .enumerate()
                .map(|(k, v)| (start + k) as f32 * v)
                .sum::<f32>()
                / weight;
            found.push(centre);
        } else {
            i += 1;
        }
    }
    found
}

/// Simple baseline straightening: measure how far the text lines in each
/// vertical strip sit above or below the same lines in the reference strip
/// (the one with the most lines), interpolate that shift across the width,
/// and move every column vertically to cancel it. Returns the image and the
/// per-strip shifts in pixels (positive = the lines sat lower than the
/// reference).
#[must_use]
pub fn dewarp(img: &GrayImage, strips: u32) -> (GrayImage, Vec<f32>) {
    let strips = strips.max(2);
    let (w, h) = img.dimensions();
    let per_strip = line_peaks(img, strips);
    // The strip with the most lines is the reference; ties go to the centre.
    let centre = per_strip.len() / 2;
    let Some(reference_index) = (0..per_strip.len())
        .max_by_key(|&i| (per_strip[i].len(), std::cmp::Reverse(i.abs_diff(centre))))
    else {
        return (img.clone(), Vec::new());
    };
    let reference = &per_strip[reference_index];
    if reference.len() < 2 {
        return (img.clone(), vec![0.0; strips as usize]);
    }
    let mut gaps: Vec<f32> = reference.windows(2).map(|p| p[1] - p[0]).collect();
    gaps.sort_by(f32::total_cmp);
    let tolerance = (gaps[gaps.len() / 2] / 2.0).max(3.0);
    let shifts: Vec<f32> = per_strip
        .iter()
        .map(|peaks| {
            let mut deltas: Vec<f32> = reference
                .iter()
                .filter_map(|r| {
                    peaks
                        .iter()
                        .map(|p| p - r)
                        .filter(|d| d.abs() <= tolerance)
                        .min_by(|a, b| a.abs().total_cmp(&b.abs()))
                })
                .collect();
            if deltas.len() < 2 {
                return 0.0;
            }
            deltas.sort_by(f32::total_cmp);
            deltas[deltas.len() / 2]
        })
        .collect();
    let strip_w = w.div_ceil(strips).max(1) as f32;
    let shift_at = |x: u32| -> f32 {
        let pos = (x as f32 + 0.5) / strip_w - 0.5;
        let i0 = pos.floor().max(0.0) as usize;
        let i0 = i0.min(shifts.len() - 1);
        let i1 = (i0 + 1).min(shifts.len() - 1);
        let a = (pos - i0 as f32).clamp(0.0, 1.0);
        shifts[i0] * (1.0 - a) + shifts[i1] * a
    };
    let mut out = GrayImage::new(w, h);
    for x in 0..w {
        let shift = shift_at(x);
        for y in 0..h {
            out.put_pixel(
                x,
                y,
                Luma([sample_bilinear(img, x as f32, y as f32 + shift)]),
            );
        }
    }
    (out, shifts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank(w: u32, h: u32) -> GrayImage {
        GrayImage::from_pixel(w, h, Luma([255]))
    }

    fn ruled(w: u32, h: u32, spacing: u32) -> GrayImage {
        let mut img = blank(w, h);
        for y in (spacing..h - spacing).step_by(spacing as usize) {
            for x in 20..w - 20 {
                for t in 0..3 {
                    img.put_pixel(x, y + t, Luma([0]));
                }
            }
        }
        img
    }

    #[test]
    fn deskew_recovers_a_known_rotation() {
        let straight = ruled(300, 240, 24);
        let skewed = rotate_gray(&straight, 2.0);
        let angle = estimate_skew(&skewed, 5.0);
        assert!(
            (angle + 2.0).abs() <= 0.15,
            "estimated {angle}, expected -2.0"
        );
        let (fixed, applied) = deskew(&skewed, 5.0);
        assert!((applied - angle).abs() < 1e-6);
        // Every strip sees the lines at the same rows again.
        let peaks = line_peaks(&fixed, 4);
        let reference = &peaks[1];
        assert!(reference.len() >= 5, "{reference:?}");
        for strip in &peaks[1..3] {
            for (a, b) in strip.iter().zip(reference) {
                assert!((a - b).abs() <= 1.5, "{strip:?} vs {reference:?}");
            }
        }
    }

    #[test]
    fn estimate_skew_is_zero_for_blank_and_straight_pages() {
        assert!(estimate_skew(&blank(50, 50), 5.0).abs() < f32::EPSILON);
        assert!(estimate_skew(&ruled(300, 240, 24), 5.0).abs() <= 0.05);
    }

    #[test]
    fn background_normalisation_removes_a_stain_and_keeps_ink() {
        let mut img = ruled(256, 128, 32);
        // A dark gradient "stain" from the left edge over the whole page.
        for y in 0..128 {
            for x in 0..256 {
                let p = img.get_pixel_mut(x, y);
                if p.0[0] == 255 {
                    p.0[0] = (140 + x / 3).min(255) as u8;
                }
            }
        }
        let clean = normalise_background(&img, 16);
        let background: Vec<u8> = clean
            .enumerate_pixels()
            .filter(|(x, y, _)| *x % 7 == 3 && *y % 32 == 16)
            .map(|(_, _, p)| p.0[0])
            .collect();
        let mean = background.iter().map(|&v| f64::from(v)).sum::<f64>() / background.len() as f64;
        assert!(mean >= 250.0, "background mean {mean}");
        assert!(clean.get_pixel(100, 33).0[0] <= 10, "ink stayed dark");
    }

    #[test]
    fn dewarp_straightens_bowed_lines() {
        let (w, h) = (400u32, 200u32);
        let mut img = blank(w, h);
        for base in [40u32, 80, 120, 160] {
            for x in 10..w - 10 {
                let t = (x as f32 - 200.0) / 200.0;
                let y = base as f32 + 6.0 * t * t;
                for k in 0..3 {
                    img.put_pixel(x, (y.round() as u32 + k).min(h - 1), Luma([0]));
                }
            }
        }
        let (fixed, shifts) = dewarp(&img, 8);
        assert_eq!(shifts.len(), 8);
        assert!(shifts[0] > 3.0 && shifts[7] > 3.0, "{shifts:?}");
        let peaks = line_peaks(&fixed, 8);
        let reference = &peaks[4];
        assert_eq!(reference.len(), 4, "{reference:?}");
        for strip in &peaks {
            assert_eq!(strip.len(), 4, "{strip:?}");
            for (a, b) in strip.iter().zip(reference) {
                assert!((a - b).abs() <= 1.5, "{strip:?} vs {reference:?}");
            }
        }
    }

    #[test]
    fn otsu_separates_two_levels() {
        let mut img = blank(10, 10);
        for x in 0..5 {
            for y in 0..10 {
                img.put_pixel(x, y, Luma([30]));
            }
        }
        let t = otsu_threshold(&img);
        assert!((30..255).contains(&t), "{t}");
    }
}
