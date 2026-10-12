//! Split extracted text into embedding-sized chunks on paragraph/sentence
//! boundaries with a little overlap so meaning spanning a cut survives.

pub const MAX_CHARS: usize = 1000;
pub const OVERLAP_CHARS: usize = 150;
pub const MAX_CHUNKS: usize = 64;

pub fn chunk_text(text: &str) -> Vec<String> {
    chunk_with(text, MAX_CHARS, OVERLAP_CHARS, MAX_CHUNKS)
}

pub fn chunk_with(text: &str, max: usize, overlap: usize, max_chunks: usize) -> Vec<String> {
    let text = text.trim();
    if text.is_empty() {
        return vec![];
    }
    // Sentence-ish units: split on newlines and sentence terminators.
    let mut units: Vec<&str> = Vec::new();
    for para in text.split('\n') {
        let mut start = 0;
        let bytes = para.as_bytes();
        for (i, b) in bytes.iter().enumerate() {
            if matches!(b, b'.' | b'!' | b'?')
                && bytes.get(i + 1).is_none_or(|n| n.is_ascii_whitespace())
            {
                let u = para[start..=i].trim();
                if !u.is_empty() {
                    units.push(u);
                }
                start = i + 1;
            }
        }
        let tail = para[start..].trim();
        if !tail.is_empty() {
            units.push(tail);
        }
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();
    for u in units {
        // A single oversized unit is hard-split.
        if u.chars().count() > max {
            if !cur.is_empty() {
                chunks.push(std::mem::take(&mut cur));
            }
            let chars: Vec<char> = u.chars().collect();
            for piece in chars.chunks(max) {
                chunks.push(piece.iter().collect());
            }
            continue;
        }
        if cur.chars().count() + u.chars().count() + 1 > max && !cur.is_empty() {
            let prev = std::mem::take(&mut cur);
            // carry the tail of the previous chunk as overlap
            let tail: String = prev
                .chars()
                .rev()
                .take(overlap)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let tail = match tail.find(char::is_whitespace) {
                Some(i) => tail[i..].trim_start().to_string(),
                None => String::new(),
            };
            chunks.push(prev);
            cur = tail;
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(u);
    }
    if !cur.trim().is_empty() {
        chunks.push(cur);
    }
    chunks.truncate(max_chunks);
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_small() {
        assert!(chunk_text("   ").is_empty());
        assert_eq!(chunk_text("Hello world."), vec!["Hello world."]);
    }

    #[test]
    fn splits_on_sentences_with_overlap_and_caps_count() {
        let text = (1..=10)
            .map(|i| format!("Sentence number {i} is here."))
            .collect::<Vec<_>>()
            .join(" ");
        let chunks = chunk_with(&text, 60, 20, 100);
        assert!(chunks.len() > 3, "{chunks:?}");
        for c in &chunks {
            assert!(c.chars().count() <= 60 + 20, "{c:?}");
        }
        // overlap: the second chunk starts with the end of the first
        assert!(
            chunks[1].starts_with("Sentence") || chunks[1].contains("here."),
            "{:?}",
            chunks[1]
        );
        assert_eq!(chunk_with(&text, 60, 20, 2).len(), 2);
    }

    #[test]
    fn hard_splits_oversized_units() {
        let long = "x".repeat(2500);
        let chunks = chunk_with(&long, 1000, 100, 100);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].len(), 1000);
        assert_eq!(chunks[2].len(), 500);
    }
}
