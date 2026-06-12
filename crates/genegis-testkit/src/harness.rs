use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// North-star prompt used as the fixed pipeline benchmark fixture.
pub const NORTH_STAR_PROMPT: &str = "名古屋市の人口密度を表示";

/// Default choropleth viewport for render benchmarks (1280×720).
pub const DEFAULT_VIEWPORT: (f32, f32) = (1280.0, 720.0);

/// Default timed iterations (excluding warmup).
pub const DEFAULT_ITERATIONS: u32 = 10;

/// Warmup iterations discarded before timing.
pub const DEFAULT_WARMUP: u32 = 2;

/// Timing statistics for one benchmark target.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkSample {
    /// Benchmark target name (`pipeline`, `render_mesh`, …).
    pub name: String,
    /// Number of timed iterations (excluding warmup).
    pub iterations: u32,
    /// Warmup iterations discarded before timing.
    pub warmup: u32,
    /// Median elapsed time in nanoseconds.
    pub median_ns: u64,
    /// Mean elapsed time in nanoseconds.
    pub mean_ns: u64,
    /// Minimum elapsed time in nanoseconds.
    pub min_ns: u64,
    /// Maximum elapsed time in nanoseconds.
    pub max_ns: u64,
}

impl BenchmarkSample {
    /// Median elapsed time in milliseconds.
    pub fn median_ms(&self) -> f64 {
        self.median_ns as f64 / 1_000_000.0
    }
}

/// Collection of benchmark samples (e.g. pipeline + render).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkReport {
    /// Individual benchmark samples.
    pub samples: Vec<BenchmarkSample>,
}

impl BenchmarkReport {
    /// Serialize the report as pretty-printed JSON.
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

/// Run `f` for warmup + timed iterations and return per-iteration durations.
pub fn time_iterations<F, T, E>(warmup: u32, iterations: u32, mut f: F) -> Result<Vec<Duration>, E>
where
    F: FnMut() -> Result<T, E>,
{
    for _ in 0..warmup {
        f()?;
    }

    let mut samples = Vec::with_capacity(iterations as usize);
    for _ in 0..iterations {
        let start = Instant::now();
        f()?;
        samples.push(start.elapsed());
    }
    Ok(samples)
}

pub(crate) fn summarize(name: &str, warmup: u32, durations: &[Duration]) -> BenchmarkSample {
    let mut nanos: Vec<u64> = durations.iter().map(|d| d.as_nanos() as u64).collect();
    nanos.sort_unstable();

    let iterations = nanos.len() as u32;
    let min_ns = *nanos.first().unwrap_or(&0);
    let max_ns = *nanos.last().unwrap_or(&0);
    let mean_ns = if iterations == 0 {
        0
    } else {
        nanos.iter().sum::<u64>() / iterations as u64
    };
    let median_ns = if iterations == 0 {
        0
    } else {
        nanos[iterations as usize / 2]
    };

    BenchmarkSample {
        name: name.to_string(),
        iterations,
        warmup,
        median_ns,
        mean_ns,
        min_ns,
        max_ns,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_computes_median_and_mean() {
        let durations = vec![
            Duration::from_nanos(100),
            Duration::from_nanos(200),
            Duration::from_nanos(300),
        ];
        let sample = summarize("demo", 1, &durations);
        assert_eq!(sample.median_ns, 200);
        assert_eq!(sample.mean_ns, 200);
        assert_eq!(sample.min_ns, 100);
        assert_eq!(sample.max_ns, 300);
    }
}
