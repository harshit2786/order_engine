use std::time::{Duration, Instant};

use hdrhistogram::Histogram;
use parking_lot::RwLock;

use crate::matching::engine::MatchingEngine;
use crate::models::metrics::MetricsResponse;

/// Tracks performance metrics for the matching engine
pub struct MetricsTracker {
    /// Histogram for latency tracking (in microseconds)
    latency_histogram: RwLock<Histogram<u64>>,
    /// When the tracker was started
    start_time: Instant,
    /// Total orders processed (for throughput calculation)
    orders_processed: RwLock<u64>,
}

impl MetricsTracker {
    pub fn new() -> Self {
        // Create histogram that can record values from 1 microsecond to 60 seconds
        // with 3 significant figures of precision
        let histogram = Histogram::new_with_bounds(1, 60_000_000, 3)
            .expect("Failed to create histogram");

        Self {
            latency_histogram: RwLock::new(histogram),
            start_time: Instant::now(),
            orders_processed: RwLock::new(0),
        }
    }

    /// Records a latency measurement
    pub fn record_latency(&self, duration: Duration) {
        let micros = duration.as_micros() as u64;
        
        // Clamp to histogram bounds
        let micros = micros.max(1).min(60_000_000);

        let mut histogram = self.latency_histogram.write();
        let _ = histogram.record(micros);

        let mut count = self.orders_processed.write();
        *count += 1;
    }

    /// Gets latency at a given percentile (returns milliseconds)
    pub fn latency_percentile(&self, percentile: f64) -> f64 {
        let histogram = self.latency_histogram.read();
        
        if histogram.len() == 0 {
            return 0.0;
        }

        let micros = histogram.value_at_percentile(percentile);
        micros as f64 / 1000.0 // Convert to milliseconds
    }

    /// Gets p50 latency in milliseconds
    pub fn latency_p50_ms(&self) -> f64 {
        self.latency_percentile(50.0)
    }

    /// Gets p99 latency in milliseconds
    pub fn latency_p99_ms(&self) -> f64 {
        self.latency_percentile(99.0)
    }

    /// Gets p999 latency in milliseconds
    pub fn latency_p999_ms(&self) -> f64 {
        self.latency_percentile(99.9)
    }

    /// Gets throughput in orders per second
    pub fn throughput_orders_per_sec(&self) -> f64 {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        
        if elapsed == 0.0 {
            return 0.0;
        }

        let count = *self.orders_processed.read();
        count as f64 / elapsed
    }

    /// Gets total orders processed
    pub fn orders_processed(&self) -> u64 {
        *self.orders_processed.read()
    }

    /// Resets the metrics (useful for testing or periodic resets)
    pub fn reset(&self) {
        let mut histogram = self.latency_histogram.write();
        histogram.reset();

        let mut count = self.orders_processed.write();
        *count = 0;
    }

    /// Generates a full metrics response
    pub fn get_metrics(&self, engine: &MatchingEngine) -> MetricsResponse {
        MetricsResponse {
            orders_received: engine.orders_received() as u64,
            orders_matched: engine.orders_matched() as u64,
            orders_cancelled: engine.orders_cancelled() as u64,
            orders_in_book: engine.orders_in_book() as u64,
            trades_executed: engine.trades_executed(),
            latency_p50_ms: self.latency_p50_ms(),
            latency_p99_ms: self.latency_p99_ms(),
            latency_p999_ms: self.latency_p999_ms(),
            throughput_orders_per_sec: self.throughput_orders_per_sec(),
        }
    }

    pub fn uptime_seconds(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }
}

impl Default for MetricsTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_new_metrics_tracker() {
        let tracker = MetricsTracker::new();
        
        assert_eq!(tracker.orders_processed(), 0);
        assert_eq!(tracker.latency_p50_ms(), 0.0);
        assert_eq!(tracker.latency_p99_ms(), 0.0);
        assert_eq!(tracker.latency_p999_ms(), 0.0);
    }

    #[test]
    fn test_record_latency() {
        let tracker = MetricsTracker::new();

        // Record some latencies
        tracker.record_latency(Duration::from_micros(1000)); // 1ms
        tracker.record_latency(Duration::from_micros(2000)); // 2ms
        tracker.record_latency(Duration::from_micros(3000)); // 3ms

        assert_eq!(tracker.orders_processed(), 3);
        
        // p50 should be around 2ms (median)
        let p50 = tracker.latency_p50_ms();
        assert!(p50 >= 1.0 && p50 <= 3.0, "p50 was {}", p50);
    }

    #[test]
    fn test_latency_percentiles() {
        let tracker = MetricsTracker::new();

        // Record 100 latencies: 1ms, 2ms, ..., 100ms
        for i in 1..=100 {
            tracker.record_latency(Duration::from_millis(i));
        }

        let p50 = tracker.latency_p50_ms();
        let p99 = tracker.latency_p99_ms();
        let p999 = tracker.latency_p999_ms();

        // p50 should be around 50ms
        assert!(p50 >= 49.0 && p50 <= 51.0, "p50 was {}", p50);
        
        // p99 should be around 99ms
        assert!(p99 >= 98.0 && p99 <= 100.0, "p99 was {}", p99);
        
        // p999 should be around 100ms (highest value)
        assert!(p999 >= 99.0 && p999 <= 101.0, "p999 was {}", p999);
    }

    #[test]
    fn test_throughput() {
        let tracker = MetricsTracker::new();

        // Record some orders
        for _ in 0..100 {
            tracker.record_latency(Duration::from_micros(100));
        }

        // Sleep a bit to get measurable elapsed time
        thread::sleep(Duration::from_millis(100));

        let throughput = tracker.throughput_orders_per_sec();
        
        // Should be roughly 100 orders / 0.1 seconds = 1000 orders/sec
        // But timing is imprecise, so just check it's positive and reasonable
        assert!(throughput > 0.0, "throughput was {}", throughput);
    }

    #[test]
    fn test_reset() {
        let tracker = MetricsTracker::new();

        // Record some latencies
        tracker.record_latency(Duration::from_millis(5));
        tracker.record_latency(Duration::from_millis(10));

        assert_eq!(tracker.orders_processed(), 2);
        assert!(tracker.latency_p50_ms() > 0.0);

        // Reset
        tracker.reset();

        assert_eq!(tracker.orders_processed(), 0);
        assert_eq!(tracker.latency_p50_ms(), 0.0);
    }

    #[test]
    fn test_extreme_latencies() {
        let tracker = MetricsTracker::new();

        // Very small latency (1 microsecond)
        tracker.record_latency(Duration::from_nanos(1000));
        
        // Very large latency (10 seconds)
        tracker.record_latency(Duration::from_secs(10));

        assert_eq!(tracker.orders_processed(), 2);
        
        // Should not panic or overflow
        let _ = tracker.latency_p50_ms();
        let _ = tracker.latency_p99_ms();
    }
}