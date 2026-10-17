//! Rank fusion for hybrid retrieval.

/// Reciprocal Rank Fusion.
///
/// RRF combines rankings from heterogeneous scorers (BM25 and cosine) without
/// needing their scores to be comparable. `k = 60` is the value from the
/// original Cormack et al. paper and is a good default.
pub fn reciprocal_rank_fusion(rankings: &[Vec<String>], k: f64) -> Vec<(String, f64)> {
    let mut scores: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    for ranking in rankings {
        for (rank, id) in ranking.iter().enumerate() {
            let contribution = 1.0 / (k + (rank as f64 + 1.0));
            *scores.entry(id.clone()).or_insert(0.0) += contribution;
        }
    }
    let mut fused: Vec<(String, f64)> = scores.into_iter().collect();
    fused.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            // Stable tie-break so results are deterministic.
            .then_with(|| a.0.cmp(&b.0))
    });
    fused
}

/// Normalised score in `0..=1` for display purposes only.
pub fn normalize_scores(fused: &[(String, f64)]) -> Vec<(String, f64)> {
    let max = fused.iter().map(|(_, s)| *s).fold(0.0f64, f64::max);
    if max <= 0.0 {
        return fused.iter().map(|(id, _)| (id.clone(), 0.0)).collect();
    }
    fused
        .iter()
        .map(|(id, s)| (id.clone(), s / max))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuses_rankings_and_prefers_agreement() {
        let fts = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let vector = vec!["b".to_string(), "a".to_string(), "d".to_string()];
        let fused = reciprocal_rank_fusion(&[fts, vector], 60.0);
        // "a" and "b" appear in both lists, so they outrank "c" and "d".
        let ids: Vec<&str> = fused.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids[0], "a");
        assert_eq!(ids[1], "b");
        assert!(ids.contains(&"c"));
        assert!(ids.contains(&"d"));
    }

    #[test]
    fn is_deterministic_for_ties() {
        let one = reciprocal_rank_fusion(&[vec!["z".into(), "y".into()]], 60.0);
        let two = reciprocal_rank_fusion(&[vec!["z".into(), "y".into()]], 60.0);
        assert_eq!(one, two);
    }

    #[test]
    fn normalization_is_bounded() {
        let fused = vec![("a".to_string(), 0.5), ("b".to_string(), 0.25)];
        let norm = normalize_scores(&fused);
        assert!((norm[0].1 - 1.0).abs() < 1e-9);
        assert!((norm[1].1 - 0.5).abs() < 1e-9);
    }
}
