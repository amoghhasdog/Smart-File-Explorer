//! Deterministic bag-of-hashed-words embedder for tests. Texts that share
//! words get similar vectors, which is enough to exercise the pipeline and
//! ranking without a model download.

use std::hash::{Hash, Hasher};

use crate::{Result, TextEmbedder};

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
            // Spread each word over three dimensions with signs.
            for k in 0..3 {
                let idx = ((x >> (k * 16)) as usize) % self.dim;
                let sign = if (x >> (48 + k)) & 1 == 0 { 1.0 } else { -1.0 };
                v[idx] += sign;
            }
        }
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        v
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
}
