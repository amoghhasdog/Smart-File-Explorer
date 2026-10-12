//! Deterministic embedders for tests.
//!
//! * [`FakeEmbedder`]: bag of hashed words — texts sharing words are close.
//! * [`FakeImageEmbedder`]: images map to their average colour; colour words
//!   in a query map to the same space, so "blue" finds blue pictures.

use std::hash::{Hash, Hasher};

use image::{DynamicImage, GenericImageView};

use crate::{ImageEmbedder, Result, TextEmbedder};

pub struct FakeEmbedder {
    dim: usize,
}

impl Default for FakeEmbedder {
    fn default() -> Self {
        Self { dim: 64 }
    }
}

impl FakeEmbedder {
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }

    fn vector(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0f32; self.dim];
        for word in text
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() > 1)
            .map(|w| w.to_lowercase())
        {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            word.hash(&mut h);
            let x = h.finish();
            for k in 0..3 {
                let idx = ((x >> (k * 16)) as usize) % self.dim;
                let sign = if (x >> (48 + k)) & 1 == 0 { 1.0 } else { -1.0 };
                v[idx] += sign;
            }
        }
        normalise(v)
    }
}

impl TextEmbedder for FakeEmbedder {
    fn model_id(&self) -> &str {
        "fake-hash-v1"
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| self.vector(t)).collect())
    }
}

#[derive(Default)]
pub struct FakeImageEmbedder;

const IMG_DIM: usize = 4;

impl ImageEmbedder for FakeImageEmbedder {
    fn model_id(&self) -> &str {
        "fake-colour-v1"
    }

    fn dim(&self) -> usize {
        IMG_DIM
    }

    fn embed_images(&self, images: Vec<DynamicImage>) -> Result<Vec<Vec<f32>>> {
        Ok(images
            .into_iter()
            .map(|img| {
                let small = img.thumbnail(8, 8).to_rgb8();
                let n = small.pixels().len().max(1) as f32;
                let (mut r, mut g, mut b) = (0f32, 0f32, 0f32);
                for p in small.pixels() {
                    r += p.0[0] as f32;
                    g += p.0[1] as f32;
                    b += p.0[2] as f32;
                }
                let _ = img.dimensions();
                normalise(vec![r / n / 255.0, g / n / 255.0, b / n / 255.0, 0.2])
            })
            .collect())
    }

    fn embed_text(&self, query: &str) -> Result<Vec<f32>> {
        let q = query.to_lowercase();
        let mut v = [0f32, 0.0, 0.0, 0.2];
        for (word, rgb) in [
            ("red", [1.0, 0.0, 0.0]),
            ("green", [0.0, 1.0, 0.0]),
            ("blue", [0.0, 0.0, 1.0]),
            ("yellow", [1.0, 1.0, 0.0]),
            ("white", [1.0, 1.0, 1.0]),
            ("bright", [1.0, 1.0, 1.0]),
        ] {
            if q.contains(word) {
                v[0] += rgb[0];
                v[1] += rgb[1];
                v[2] += rgb[2];
            }
        }
        Ok(normalise(v.to_vec()))
    }
}

fn normalise(mut v: Vec<f32>) -> Vec<f32> {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cosine;

    #[test]
    fn similar_texts_are_closer_than_unrelated_ones() {
        let e = FakeEmbedder::default();
        let v = e
            .embed(&[
                "the athena project roadmap",
                "athena project planning notes",
                "recipe for banana bread",
            ])
            .unwrap();
        assert_eq!(v[0].len(), 64);
        let close = cosine(&v[0], &v[1]);
        let far = cosine(&v[0], &v[2]);
        assert!(close > far, "{close} vs {far}");
        assert!((cosine(&v[0], &v[0]) - 1.0).abs() < 1e-5);
        assert_eq!(e.embed(&[""]).unwrap()[0], vec![0.0; 64]);
    }

    #[test]
    fn colour_words_find_matching_images() {
        let e = FakeImageEmbedder;
        let blue =
            DynamicImage::ImageRgb8(image::RgbImage::from_pixel(4, 4, image::Rgb([10, 20, 240])));
        let red =
            DynamicImage::ImageRgb8(image::RgbImage::from_pixel(4, 4, image::Rgb([240, 20, 10])));
        let v = e.embed_images(vec![blue, red]).unwrap();
        let q = e.embed_text("a lot of blue sky").unwrap();
        assert!(cosine(&q, &v[0]) > cosine(&q, &v[1]));
        assert_eq!(e.dim(), 4);
    }
}
