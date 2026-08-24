//! Raster band algebra (RFC 0005 platform gap: index computation).
//!
//! Minimal, fail-closed band math over full scenes. NDVI follows the
//! standard `(nir − red) / (nir + red)` formulation; zero-total pixels
//! (deep water, nodata) map to 0.0 by convention and every output is
//! clamped to `[-1, 1]` so downstream range checks can treat any
//! excursion as corruption.

use crate::error::RasterError;

/// Compute NDVI from red and NIR bands of equal length.
///
/// Values are reflectances in any linear scale (DN or float); only the
/// ratio matters. Returns one value per pixel in `[-1, 1]`.
pub fn ndvi(red: &[f64], nir: &[f64]) -> Result<Vec<f64>, RasterError> {
    if red.len() != nir.len() {
        return Err(RasterError::Invalid(format!(
            "band length mismatch: red={}, nir={}",
            red.len(),
            nir.len()
        )));
    }
    let mut values = Vec::with_capacity(red.len());
    for (&r, &n) in red.iter().zip(nir.iter()) {
        if !r.is_finite() || !n.is_finite() {
            return Err(RasterError::Invalid("NDVI inputs must be finite".into()));
        }
        let total = n + r;
        let value = if total.abs() < f64::EPSILON {
            0.0
        } else {
            ((n - r) / total).clamp(-1.0, 1.0)
        };
        values.push(value);
    }
    Ok(values)
}

/// Mean of finite NDVI values; `None` for an empty slice.
pub fn mean(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    Some(values.iter().sum::<f64>() / values.len() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ndvi_matches_reference_ratios() {
        // pure vegetation: nir >> red → near +0.6; bare soil: balanced.
        let red = vec![46.0, 115.0];
        let nir = vec![184.0, 115.0];
        let out = ndvi(&red, &nir).expect("ndvi");
        assert!((out[0] - 0.6).abs() < 1e-9);
        assert!((out[1]).abs() < 1e-12);
    }

    #[test]
    fn ndvi_is_bounded_and_handles_zero_total() {
        let red = vec![0.0, 10.0, 10.0];
        let nir = vec![0.0, -5.0, 30.0];
        let out = ndvi(&red, &nir).expect("ndvi");
        // Degenerate zero-total pixel maps to the neutral convention.
        assert_eq!(out[0], 0.0);
        // Out-of-range ratio clamps to the physical bound.
        assert_eq!(out[1], -1.0);
        assert!((out[2] - (20.0 / 40.0)).abs() < 1e-12);
        assert!(out.iter().all(|v| (-1.0..=1.0).contains(v)));
    }

    #[test]
    fn ndvi_rejects_mismatched_bands() {
        assert!(ndvi(&[0.1, 0.2], &[0.3]).is_err());
    }

    #[test]
    fn mean_of_empty_is_none() {
        assert!(mean(&[]).is_none());
        assert!((mean(&[0.2, 0.4]).unwrap() - 0.3).abs() < 1e-12);
    }
}
