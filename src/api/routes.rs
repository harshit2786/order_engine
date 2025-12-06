use axum::{
    routing::{delete, get, post},
    Router,
};

use crate::api::handlers;
use crate::state::AppState;

/// Creates the API router with all routes
pub fn create_router(state: AppState) -> Router {
    Router::new()
        // Order endpoints
        .route("/api/v1/orders", post(handlers::submit_order))
        .route("/api/v1/orders/:order_id", get(handlers::get_order_status))
        .route("/api/v1/orders/:order_id", delete(handlers::cancel_order))
        // Order book endpoint
        .route("/api/v1/orderbook/:symbol", get(handlers::get_order_book))
        // Metrics endpoint
        .route("/metrics", get(handlers::get_metrics))
        // Health check
        .route("/health", get(handlers::health_check))
        // Attach shared state
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_health_check_route() {
        let state = AppState::new();
        let app = create_router(state);

        let request = Request::builder()
            .uri("/health")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_metrics_route() {
        let state = AppState::new();
        let app = create_router(state);

        let request = Request::builder()
            .uri("/metrics")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}