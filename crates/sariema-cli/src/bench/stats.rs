// SPDX-License-Identifier: Apache-2.0

use std::cmp::Ordering;

use serde::Serialize;
use serde_json::{Value, json};

/// Nearest-rank percentile `p` (0 to 1) of `sorted`, which is not empty.
fn rank<T: Copy>(sorted: &[T], p: f64) -> T {
    let k = (p * sorted.len() as f64).ceil() as usize;
    sorted[k.clamp(1, sorted.len()) - 1]
}

/// Median, 90th and 99th percentiles, minimum and maximum; null when empty.
pub fn summary<T: Copy + PartialOrd + Serialize>(values: &[T]) -> Value {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let (Some(&min), Some(&max)) = (sorted.first(), sorted.last()) else {
        return Value::Null;
    };
    json!({
        "median": rank(&sorted, 0.5),
        "p90": rank(&sorted, 0.9),
        "p99": rank(&sorted, 0.99),
        "min": min,
        "max": max,
    })
}

pub fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    if sorted.is_empty() {
        return 0.0;
    }
    rank(&sorted, 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_percentiles() {
        let values: Vec<f64> = (1..=100).rev().map(f64::from).collect();
        let s = summary(&values);
        assert_eq!(s["median"], 50.0);
        assert_eq!(s["p90"], 90.0);
        assert_eq!(s["p99"], 99.0);
        assert_eq!(
            (s["min"].as_f64(), s["max"].as_f64()),
            (Some(1.0), Some(100.0))
        );
        assert_eq!(summary::<u64>(&[]), Value::Null);
        assert_eq!(summary(&[7u64, 9])["max"], 9);
        assert_eq!(median(&[3.0]), 3.0);
    }
}
