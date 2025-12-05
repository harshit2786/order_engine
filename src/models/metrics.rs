use serde::Serialize;

/// Response for GET /metrics endpoint
#[derive(Debug, Clone, Serialize)]
pub struct MetricsResponse {
    pub orders_received: u64,
    pub orders_matched: u64,
    pub orders_cancelled: u64,
    pub orders_in_book: u64,
    pub trades_executed: u64,
    pub latency_p50_ms: f64,
    pub latency_p99_ms: f64,
    pub latency_p999_ms: f64,
    pub throughput_orders_per_sec: f64,
}

impl Default for MetricsResponse {
    fn default() -> Self {
        Self {
            orders_received: 0,
            orders_matched: 0,
            orders_cancelled: 0,
            orders_in_book: 0,
            trades_executed: 0,
            latency_p50_ms: 0.0,
            latency_p99_ms: 0.0,
            latency_p999_ms: 0.0,
            throughput_orders_per_sec: 0.0,
        }
    }
}