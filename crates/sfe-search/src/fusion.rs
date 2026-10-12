//! Reciprocal rank fusion: combine several ranked lists without needing
//! comparable scores. `score(d) = Σ_lists 1 / (k + rank(d))`.

use sfe_core::FileId;

pub const DEFAULT_K: f64 = 60.0;

/// Fuse ranked id lists (best first). Output is sorted best first; ties keep
/// the order of first appearance.
pub fn rrf(lists: &[Vec<FileId>], k: f64) -> Vec<(FileId, f64)> {
    let mut scores: Vec<(FileId, f64)> = Vec::new();
    let mut index = std::collections::HashMap::<FileId, usize>::new();
    for list in lists {
        for (rank, id) in list.iter().enumerate() {
            let add = 1.0 / (k + rank as f64 + 1.0);
            match index.get(id) {
                Some(&i) => scores[i].1 += add,
                None => {
                    index.insert(*id, scores.len());
                    scores.push((*id, add));
                }
            }
        }
    }
    scores.sort_by(|a, b| b.1.total_cmp(&a.1));
    scores
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[i64]) -> Vec<FileId> {
        v.iter().map(|&i| FileId(i)).collect()
    }

    #[test]
    fn single_list_keeps_order() {
        let out = rrf(&[ids(&[3, 1, 2])], DEFAULT_K);
        let order: Vec<_> = out.iter().map(|(id, _)| id.0).collect();
        assert_eq!(order, vec![3, 1, 2]);
    }

    #[test]
    fn documents_in_both_lists_rise() {
        let out = rrf(&[ids(&[1, 2, 3]), ids(&[3, 4, 1])], DEFAULT_K);
        let order: Vec<_> = out.iter().map(|(id, _)| id.0).collect();
        // 1 and 3 appear in both; 1 has ranks (1,3), 3 has (3,1) → equal; first seen wins.
        assert_eq!(&order[..2], &[1, 3]);
        assert_eq!(&order[2..], &[2, 4]);
    }

    #[test]
    fn empty_input() {
        assert!(rrf(&[], DEFAULT_K).is_empty());
        assert!(rrf(&[vec![]], DEFAULT_K).is_empty());
    }
}
