// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Small statistics helpers for the stress scenarios: summaries of latencies and a least-squares
//! slope (the RSS growth per iteration in T04). Pure functions, unit-tested.

use serde_json::{Value, json};

/// Arithmetic mean; `None` for an empty slice.
pub fn mean(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        None
    } else {
        Some(values.iter().sum::<f64>() / values.len() as f64)
    }
}

/// The `p`-th percentile (0..=100) with linear interpolation between the closest ranks; `None` for an empty slice.
pub fn percentile(values: &[f64], p: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted: Vec<f64> = values.iter().copied().filter(|v| !v.is_nan()).collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(|a, b| a.total_cmp(b));
    let p = p.clamp(0.0, 100.0);
    let rank = p / 100.0 * (sorted.len() - 1) as f64;
    let low = rank.floor() as usize;
    let high = rank.ceil() as usize;
    let fraction = rank - low as f64;
    Some(sorted[low] + (sorted[high] - sorted[low]) * fraction)
}

pub fn median(values: &[f64]) -> Option<f64> {
    percentile(values, 50.0)
}

pub fn max(values: &[f64]) -> Option<f64> {
    values
        .iter()
        .copied()
        .filter(|v| !v.is_nan())
        .reduce(f64::max)
}

pub fn min(values: &[f64]) -> Option<f64> {
    values
        .iter()
        .copied()
        .filter(|v| !v.is_nan())
        .reduce(f64::min)
}

/// Least-squares slope of `y` against `x`. `None` for fewer than two points or when all `x` are equal.
pub fn slope(x: &[f64], y: &[f64]) -> Option<f64> {
    let n = x.len().min(y.len());
    if n < 2 {
        return None;
    }
    let mean_x = x[..n].iter().sum::<f64>() / n as f64;
    let mean_y = y[..n].iter().sum::<f64>() / n as f64;
    let mut covariance = 0.0;
    let mut variance = 0.0;
    for i in 0..n {
        covariance += (x[i] - mean_x) * (y[i] - mean_y);
        variance += (x[i] - mean_x) * (x[i] - mean_x);
    }
    if variance == 0.0 {
        None
    } else {
        Some(covariance / variance)
    }
}

/// Slope of `y` against its own index (0, 1, 2, ...): "growth per iteration".
pub fn slope_per_index(y: &[f64]) -> Option<f64> {
    let x: Vec<f64> = (0..y.len()).map(|i| i as f64).collect();
    slope(&x, y)
}

/// `{n, min, mean, median, p95, max}` of `values` (all in the unit the caller uses), rounded to 3 decimals.
pub fn summary_json(values: &[f64]) -> Value {
    if values.is_empty() {
        return json!({ "n": 0 });
    }
    json!({
        "n": values.len(),
        "min": round3(min(values)),
        "mean": round3(mean(values)),
        "median": round3(median(values)),
        "p95": round3(percentile(values, 95.0)),
        "max": round3(max(values)),
    })
}

/// Rounds to three decimals so reports stay readable.
pub fn round3(value: Option<f64>) -> Value {
    match value {
        Some(v) if v.is_finite() => json!((v * 1000.0).round() / 1000.0),
        _ => Value::Null,
    }
}

/// Milliseconds with three decimals, as a JSON number.
pub fn ms(duration: std::time::Duration) -> f64 {
    (duration.as_secs_f64() * 1_000_000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mean_median_percentiles() {
        let v = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(mean(&v), Some(3.0));
        assert_eq!(median(&v), Some(3.0));
        assert_eq!(percentile(&v, 0.0), Some(1.0));
        assert_eq!(percentile(&v, 100.0), Some(5.0));
        assert_eq!(percentile(&v, 25.0), Some(2.0));
        assert_eq!(max(&v), Some(5.0));
        assert_eq!(min(&v), Some(1.0));
        assert_eq!(mean(&[]), None);
        assert_eq!(percentile(&[], 50.0), None);
    }

    #[test]
    fn percentile_interpolates_and_ignores_nan() {
        assert_eq!(percentile(&[10.0, 20.0], 50.0), Some(15.0));
        assert_eq!(percentile(&[f64::NAN, 4.0], 50.0), Some(4.0));
    }

    #[test]
    fn slope_of_a_line_and_of_noise_free_growth() {
        let y: Vec<f64> = (0..10).map(|i| 100.0 + 3.5 * i as f64).collect();
        let s = slope_per_index(&y).unwrap();
        assert!((s - 3.5).abs() < 1e-9, "slope {s}");
        assert_eq!(slope_per_index(&[5.0]), None);
        assert_eq!(slope(&[1.0, 1.0], &[1.0, 2.0]), None);
        let flat = slope_per_index(&[7.0; 20]).unwrap();
        assert!(flat.abs() < 1e-12);
    }

    #[test]
    fn summary_has_the_documented_keys() {
        let value = summary_json(&[1.0, 2.0, 3.0]);
        for key in ["n", "min", "mean", "median", "p95", "max"] {
            assert!(value.get(key).is_some(), "missing {key}");
        }
        assert_eq!(summary_json(&[]), json!({ "n": 0 }));
        assert_eq!(round3(Some(1.23456)), json!(1.235));
        assert_eq!(round3(Some(f64::NAN)), Value::Null);
    }

    #[test]
    fn ms_converts_with_three_decimals() {
        assert_eq!(ms(std::time::Duration::from_micros(1500)), 1.5);
    }
}
