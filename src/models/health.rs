use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub uptime_seconds: u64,
    pub orders_processed: u64,
}

impl HealthResponse {
    pub fn healthy(uptime_seconds: u64, orders_processed: u64) -> Self {
        Self {
            status: "healthy".to_string(),
            uptime_seconds,
            orders_processed,
        }
    }
}