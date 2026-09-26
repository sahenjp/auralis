use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;

const BUCKET_WIDTH_NS: u64 = 10_000;
const BUCKET_COUNT: usize = 4_001;

#[derive(Clone)]
pub struct InferenceTimingHandle {
    inner: Arc<InferenceTiming>,
}

impl Default for InferenceTimingHandle {
    fn default() -> Self {
        Self {
            inner: Arc::new(InferenceTiming::default()),
        }
    }
}

impl InferenceTimingHandle {
    pub(crate) fn record_ns(&self, elapsed_ns: u64) {
        self.inner.record_ns(elapsed_ns);
    }

    pub(crate) fn record_failure(&self) {
        self.inner.failure_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> InferenceTimingSnapshot {
        self.inner.snapshot()
    }

    pub(crate) fn reset(&self) {
        self.inner.reset();
    }
}

struct InferenceTiming {
    buckets: [AtomicU64; BUCKET_COUNT],
    count: AtomicU64,
    total_ns: AtomicU64,
    maximum_ns: AtomicU64,
    overflow_count: AtomicU64,
    failure_count: AtomicU64,
}

impl Default for InferenceTiming {
    fn default() -> Self {
        Self {
            buckets: std::array::from_fn(|_| AtomicU64::new(0)),
            count: AtomicU64::new(0),
            total_ns: AtomicU64::new(0),
            maximum_ns: AtomicU64::new(0),
            overflow_count: AtomicU64::new(0),
            failure_count: AtomicU64::new(0),
        }
    }
}

impl InferenceTiming {
    fn record_ns(&self, elapsed_ns: u64) {
        let bucket = usize::try_from(elapsed_ns / BUCKET_WIDTH_NS).unwrap_or(usize::MAX);
        let bucket = if bucket < BUCKET_COUNT {
            bucket
        } else {
            self.overflow_count.fetch_add(1, Ordering::Relaxed);
            BUCKET_COUNT - 1
        };
        self.buckets[bucket].fetch_add(1, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
        self.total_ns.fetch_add(elapsed_ns, Ordering::Relaxed);
        self.maximum_ns.fetch_max(elapsed_ns, Ordering::Relaxed);
    }

    fn snapshot(&self) -> InferenceTimingSnapshot {
        let count = self.count.load(Ordering::Relaxed);
        let total_ns = self.total_ns.load(Ordering::Relaxed);
        InferenceTimingSnapshot {
            count,
            mean_ms: if count == 0 {
                0.0
            } else {
                total_ns as f64 / count as f64 / 1_000_000.0
            },
            p50_ms: self.percentile_ms(count, 0.50),
            p95_ms: self.percentile_ms(count, 0.95),
            p99_ms: self.percentile_ms(count, 0.99),
            maximum_ms: self.maximum_ns.load(Ordering::Relaxed) as f64 / 1_000_000.0,
            histogram_bucket_width_us: BUCKET_WIDTH_NS as f64 / 1_000.0,
            histogram_maximum_ms: (BUCKET_COUNT - 1) as f64 * BUCKET_WIDTH_NS as f64 / 1_000_000.0,
            histogram_overflow_count: self.overflow_count.load(Ordering::Relaxed),
            failure_count: self.failure_count.load(Ordering::Relaxed),
        }
    }

    fn reset(&self) {
        for bucket in &self.buckets {
            bucket.store(0, Ordering::Relaxed);
        }
        self.count.store(0, Ordering::Relaxed);
        self.total_ns.store(0, Ordering::Relaxed);
        self.maximum_ns.store(0, Ordering::Relaxed);
        self.overflow_count.store(0, Ordering::Relaxed);
        self.failure_count.store(0, Ordering::Relaxed);
    }

    fn percentile_ms(&self, count: u64, percentile: f64) -> f64 {
        if count == 0 {
            return 0.0;
        }
        let target = (count as f64 * percentile).ceil() as u64;
        let mut cumulative = 0_u64;
        for (index, bucket) in self.buckets.iter().enumerate() {
            cumulative = cumulative.saturating_add(bucket.load(Ordering::Relaxed));
            if cumulative >= target {
                return index as f64 * BUCKET_WIDTH_NS as f64 / 1_000_000.0;
            }
        }
        self.maximum_ns.load(Ordering::Relaxed) as f64 / 1_000_000.0
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct InferenceTimingSnapshot {
    pub count: u64,
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub maximum_ms: f64,
    pub histogram_bucket_width_us: f64,
    pub histogram_maximum_ms: f64,
    pub histogram_overflow_count: u64,
    pub failure_count: u64,
}

#[cfg(test)]
mod tests {
    use super::InferenceTimingHandle;

    #[test]
    fn bounded_histogram_reports_percentiles_and_overflow() {
        let timing = InferenceTimingHandle::default();
        for elapsed_ns in [10_000, 20_000, 30_000, 40_000, 50_000, 100_000_000] {
            timing.record_ns(elapsed_ns);
        }
        let snapshot = timing.snapshot();
        assert_eq!(snapshot.count, 6);
        assert_eq!(snapshot.p50_ms, 0.03);
        assert_eq!(snapshot.p95_ms, 40.0);
        assert_eq!(snapshot.maximum_ms, 100.0);
        assert_eq!(snapshot.histogram_overflow_count, 1);
    }
}
