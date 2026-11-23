//! Weighted reciprocal rank fusion (RRF) of a vector list and a keyword list.
//!
//! Each list is cut by its own threshold, then a chunk scores
//! `vector_weight / (k + vector_rank) + keyword_weight / (k + keyword_rank)`, with ranks
//! from 1 and a missing list contributing nothing. The result keeps both raw scores, so
//! the UI can show why a chunk ranked where it did.

use crate::config::SearchConfig;
use crate::store::Hit;
use std::collections::HashMap;

/// One candidate: a stable key (store row id or record index), its hit, and its raw score.
pub struct Candidate {
    pub key: u64,
    pub hit: Hit,
}

pub fn rrf(vector: Vec<Candidate>, keyword: Vec<Candidate>, cfg: &SearchConfig) -> Vec<Hit> {
    let pass = |min: Option<f32>| move |c: &Candidate| min.is_none_or(|m| c.hit.score >= m);
    let vector: Vec<Candidate> = vector.into_iter().filter(pass(cfg.min_vector_score)).collect();
    let keyword: Vec<Candidate> = keyword.into_iter().filter(pass(cfg.min_keyword_score)).collect();
    let mut fused: HashMap<u64, Hit> = HashMap::new();
    for (list, weight, is_vector) in [(vector, cfg.vector_weight, true), (keyword, cfg.keyword_weight, false)] {
        for (rank, c) in list.into_iter().enumerate() {
            let part = weight / (cfg.rrf_k + rank as f32 + 1.0);
            let raw = c.hit.score;
            let entry = fused.entry(c.key).or_insert_with(|| Hit { score: 0.0, vector_score: None, keyword_score: None, ..c.hit });
            entry.score += part;
            if is_vector {
                entry.vector_score = Some(raw);
            } else {
                entry.keyword_score = Some(raw);
            }
        }
    }
    let mut hits: Vec<Hit> = fused.into_values().filter(|h| cfg.min_score.is_none_or(|m| h.score >= m)).collect();
    hits.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.text.cmp(&b.text)));
    hits.truncate(cfg.top_k);
    hits
}

/// A single list (vector-only or keyword-only search): threshold, then top k.
pub fn single(list: Vec<Hit>, min_list: Option<f32>, cfg: &SearchConfig) -> Vec<Hit> {
    let min = [min_list, cfg.min_score].into_iter().flatten().reduce(f32::max);
    let mut hits: Vec<Hit> = list.into_iter().filter(|h| min.is_none_or(|m| h.score >= m)).collect();
    hits.truncate(cfg.top_k);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(key: u64, text: &str, score: f32) -> Candidate {
        Candidate { key, hit: Hit { source: "s".into(), page: None, text: text.into(), score, vector_score: None, keyword_score: None } }
    }

    #[test]
    fn a_chunk_in_both_lists_beats_one_in_a_single_list() {
        let cfg = SearchConfig::default();
        let hits = rrf(vec![cand(1, "a", 0.9), cand(2, "b", 0.8)], vec![cand(2, "b", 12.0), cand(3, "c", 10.0)], &cfg);
        assert_eq!(hits[0].text, "b");
        assert_eq!(hits[0].vector_score, Some(0.8));
        assert_eq!(hits[0].keyword_score, Some(12.0));
        assert_eq!(hits.len(), 3);
    }

    #[test]
    fn weights_and_thresholds_apply() {
        let cfg = SearchConfig { vector_weight: 0.0, min_keyword_score: Some(11.0), ..SearchConfig::default() };
        let hits = rrf(vec![cand(1, "a", 0.9)], vec![cand(2, "b", 12.0), cand(3, "c", 10.0)], &cfg);
        assert_eq!(hits[0].text, "b");
        assert!(hits.iter().all(|h| h.text != "c"));
        let cfg = SearchConfig { min_score: Some(1.0), ..SearchConfig::default() };
        assert!(rrf(vec![cand(1, "a", 0.9)], vec![], &cfg).is_empty());
    }
}
