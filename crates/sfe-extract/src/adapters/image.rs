//! Images: dimensions (header only), EXIF (taken-at, camera, GPS) and, for
//! files under the decode limit, colour statistics — a 12-bin hue histogram
//! and the top dominant colours — that power queries like "a lot of blue".

use std::io::{BufReader, Read, Seek, SeekFrom};

use image::{DynamicImage, GenericImageView, ImageReader};
use sfe_core::FileMeta;

use crate::{ExtractError, ExtractInput, Extracted, Extractor, FileProbe, Limits, Result, Support};

pub struct Image;

const IMAGE_EXTS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "bmp", "webp", "tif", "tiff", "ico",
];

impl Extractor for Image {
    fn id(&self) -> &'static str {
        "image"
    }

    fn version(&self) -> u32 {
        1
    }

    fn supports(&self, probe: &FileProbe<'_>) -> Support {
        match probe.mime {
            Some(m) if m.starts_with("image/") && m != "image/svg+xml" => Support::ByMagic,
            Some(_) => Support::No,
            None if IMAGE_EXTS.contains(&probe.ext) => Support::ByExt,
            None => Support::No,
        }
    }

    fn extract(&self, input: &mut dyn ExtractInput, limits: &Limits) -> Result<Extracted> {
        let mut meta = FileMeta::default();
        let size = input.size();

        input.seek(SeekFrom::Start(0))?;
        let mut exif_reader = BufReader::new(ReadSeek(input));
        if let Ok(exif) = exif::Reader::new().read_from_container(&mut exif_reader) {
            apply_exif(&exif, &mut meta);
        }
        let input = exif_reader.into_inner().0;

        input.seek(SeekFrom::Start(0))?;
        let reader = ImageReader::new(BufReader::new(ReadSeek(input)))
            .with_guessed_format()
            .map_err(|_| ExtractError::Unsupported("unknown image format"))?;
        if reader.format().is_none() {
            return Err(ExtractError::Unsupported("unknown image format"));
        }

        if size <= limits.max_decode_bytes {
            let img = reader
                .decode()
                .map_err(|e| ExtractError::Parse(e.to_string()))?;
            let (w, h) = img.dimensions();
            meta.width = Some(w);
            meta.height = Some(h);
            let stats = colour_stats(&img);
            meta.hue_hist = Some(stats.hue_hist.to_vec());
            meta.dominant_colors = Some(stats.dominant_json());
        } else {
            let (w, h) = reader
                .into_dimensions()
                .map_err(|e| ExtractError::Parse(e.to_string()))?;
            meta.width = Some(w);
            meta.height = Some(h);
        }
        Ok(Extracted {
            text: None,
            meta,
            truncated: false,
        })
    }
}

fn apply_exif(exif: &exif::Exif, meta: &mut FileMeta) {
    use exif::{In, Tag, Value};
    if let Some(f) = exif
        .get_field(Tag::DateTimeOriginal, In::PRIMARY)
        .or_else(|| exif.get_field(Tag::DateTime, In::PRIMARY))
    {
        let s = f.display_value().to_string();
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S") {
            meta.taken_at = Some(dt.and_utc().timestamp());
        }
    }
    if let Some(f) = exif.get_field(Tag::Make, In::PRIMARY) {
        let make = f
            .display_value()
            .to_string()
            .trim_matches('"')
            .trim()
            .to_string();
        if !make.is_empty() {
            meta.camera_make = Some(make);
        }
    }
    let coord = |tag: Tag, ref_tag: Tag| -> Option<f64> {
        let v = exif.get_field(tag, In::PRIMARY)?;
        let r = exif.get_field(ref_tag, In::PRIMARY)?;
        let Value::Rational(parts) = &v.value else {
            return None;
        };
        if parts.len() < 3 {
            return None;
        }
        let deg = parts[0].to_f64() + parts[1].to_f64() / 60.0 + parts[2].to_f64() / 3600.0;
        let sign = match r.display_value().to_string().as_str() {
            "S" | "W" => -1.0,
            _ => 1.0,
        };
        Some(deg * sign)
    };
    meta.gps_lat = coord(Tag::GPSLatitude, Tag::GPSLatitudeRef);
    meta.gps_lon = coord(Tag::GPSLongitude, Tag::GPSLongitudeRef);
}

/// Colour summary computed on a small downsample.
#[derive(Debug, Clone, PartialEq)]
pub struct ColourStats {
    /// 12 hue bins (30° each, starting at red), each 0..=255 = share of
    /// *saturated* pixels; grey/black/white pixels do not vote.
    pub hue_hist: [u8; 12],
    /// `(r, g, b, share)` top colours, share in 0..=1, after 4-bit quantisation.
    pub dominant: Vec<(u8, u8, u8, f32)>,
}

impl ColourStats {
    pub fn dominant_json(&self) -> String {
        let items: Vec<String> = self
            .dominant
            .iter()
            .map(|(r, g, b, s)| format!("[{r},{g},{b},{s:.3}]"))
            .collect();
        format!("[{}]", items.join(","))
    }
}

pub fn colour_stats(img: &DynamicImage) -> ColourStats {
    let small = img.thumbnail(64, 64).to_rgb8();
    let mut hue_acc = [0f64; 12];
    let mut hue_total = 0f64;
    let mut buckets = std::collections::HashMap::<(u8, u8, u8), u32>::new();
    for p in small.pixels() {
        let [r, g, b] = p.0;
        *buckets.entry((r >> 4, g >> 4, b >> 4)).or_default() += 1;
        let (h, s, v) = rgb_to_hsv(r, g, b);
        // Weight by saturation × value so vivid pixels dominate, dull ones barely count.
        let w = s * v;
        if w < 0.1 {
            continue;
        }
        let bin = ((h / 30.0).floor() as usize).min(11);
        hue_acc[bin] += w;
        hue_total += w;
    }
    let mut hue_hist = [0u8; 12];
    if hue_total > 0.0 {
        for (i, v) in hue_acc.iter().enumerate() {
            hue_hist[i] = ((v / hue_total) * 255.0).round() as u8;
        }
    }
    let total = small.pixels().len().max(1) as f32;
    let mut dominant: Vec<_> = buckets
        .into_iter()
        .map(|((r, g, b), n)| (r << 4 | 8, g << 4 | 8, b << 4 | 8, n as f32 / total))
        .collect();
    dominant.sort_by(|a, b| b.3.total_cmp(&a.3));
    dominant.truncate(5);
    ColourStats { hue_hist, dominant }
}

/// Hue in degrees [0, 360), saturation and value in [0, 1].
pub fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    let (r, g, b) = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d) % 6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let h = if h < 0.0 { h + 360.0 } else { h };
    let s = if max == 0.0 { 0.0 } else { d / max };
    (h, s, max)
}

struct ReadSeek<'a>(&'a mut dyn ExtractInput);

impl Read for ReadSeek<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Seek for ReadSeek<'_> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::MemInput;

    pub(crate) fn png(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb(rgb));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn hsv_conversion() {
        assert_eq!(rgb_to_hsv(255, 0, 0).0, 0.0);
        assert!((rgb_to_hsv(0, 255, 0).0 - 120.0).abs() < 1e-9);
        assert!((rgb_to_hsv(0, 0, 255).0 - 240.0).abs() < 1e-9);
        assert_eq!(rgb_to_hsv(128, 128, 128).1, 0.0);
    }

    #[test]
    fn blue_image_lands_in_the_blue_bin() {
        let e = Image
            .extract(
                &mut MemInput::new(png(40, 30, [0, 0, 255])),
                &Limits::default(),
            )
            .unwrap();
        assert_eq!((e.meta.width, e.meta.height), (Some(40), Some(30)));
        let hist = e.meta.hue_hist.unwrap();
        assert_eq!(hist[8], 255, "240° → bin 8, got {hist:?}");
        assert!(
            e.meta
                .dominant_colors
                .unwrap()
                .starts_with("[[8,8,248,1.000]")
        );
        assert!(e.text.is_none());
    }

    #[test]
    fn grey_image_has_empty_hue_histogram() {
        let e = Image
            .extract(
                &mut MemInput::new(png(8, 8, [128, 128, 128])),
                &Limits::default(),
            )
            .unwrap();
        assert_eq!(e.meta.hue_hist.unwrap(), vec![0u8; 12]);
    }

    #[test]
    fn large_images_get_dimensions_only() {
        let limits = Limits {
            max_decode_bytes: 10,
            ..Default::default()
        };
        let e = Image
            .extract(&mut MemInput::new(png(5, 7, [1, 2, 3])), &limits)
            .unwrap();
        assert_eq!((e.meta.width, e.meta.height), (Some(5), Some(7)));
        assert!(e.meta.hue_hist.is_none());
    }

    #[test]
    fn rejects_non_images() {
        let err = Image
            .extract(
                &mut MemInput::new(b"not an image".to_vec()),
                &Limits::default(),
            )
            .unwrap_err();
        assert!(matches!(err, ExtractError::Unsupported(_)), "{err}");
    }
}
