use std::time::Instant;

use axum::{ extract::{ Path, Query, State }, http::StatusCode, Json };
use serde::Deserialize;
use uuid::Uuid;

use crate::error::AppError;
use crate::models::metrics::MetricsResponse;
use crate::models::order::{
    CancelOrderResponse,
    CreateOrderRequest,
    OrderStatusResponse,
    SubmitOrderResponse,
};
use crate::models::orderbook::OrderBookResponse;
use crate::state::AppState;
use crate::models::health::HealthResponse;

/// POST /api/v1/orders
/// Submit a new order
pub async fn submit_order(
    State(state): State<AppState>,
    Json(request): Json<CreateOrderRequest>
) -> Result<(StatusCode, Json<SubmitOrderResponse>), AppError> {
    let start = Instant::now();

    let response = state.engine.submit_order(request)?;

    // Record latency
    state.metrics.record_latency(start.elapsed());

    // Determine status code based on response type
    let status_code = match &response {
        SubmitOrderResponse::Accepted(_) => StatusCode::CREATED,
        SubmitOrderResponse::PartialFill(_) => StatusCode::ACCEPTED,
        SubmitOrderResponse::Filled(_) => StatusCode::OK,
    };

    Ok((status_code, Json(response)))
}

/// DELETE /api/v1/orders/:order_id
/// Cancel an order
pub async fn cancel_order(
    State(state): State<AppState>,
    Path(order_id): Path<Uuid>
) -> Result<Json<CancelOrderResponse>, AppError> {
    let response = state.engine.cancel_order(order_id)?;
    Ok(Json(response))
}

/// GET /api/v1/orders/:order_id
/// Get order status
pub async fn get_order_status(
    State(state): State<AppState>,
    Path(order_id): Path<Uuid>
) -> Result<Json<OrderStatusResponse>, AppError> {
    let response = state.engine.get_order(order_id)?;
    Ok(Json(response))
}

/// Query parameters for order book endpoint
#[derive(Debug, Deserialize)]
pub struct OrderBookQuery {
    /// Number of price levels to return (default: 10)
    #[serde(default = "default_depth")]
    pub depth: usize,
}

fn default_depth() -> usize {
    10
}

/// GET /api/v1/orderbook/:symbol
/// Get order book for a symbol
pub async fn get_order_book(
    State(state): State<AppState>,
    Path(symbol): Path<String>,
    Query(query): Query<OrderBookQuery>
) -> Json<OrderBookResponse> {
    let response = state.engine.get_order_book(&symbol, query.depth);
    Json(response)
}

/// GET /metrics
/// Get system metrics
pub async fn get_metrics(State(state): State<AppState>) -> Json<MetricsResponse> {
    let response = state.metrics.get_metrics(&state.engine);
    Json(response)
}

/// GET /health
/// Health check endpoint
pub async fn health_check(State(state): State<AppState>) -> Json<HealthResponse> {
    let uptime_seconds = state.metrics.uptime_seconds();
    let orders_processed = state.engine.orders_received() as u64;

    Json(HealthResponse::healthy(uptime_seconds, orders_processed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::order::{ OrderType, Side };
    use axum::{ body::Body, http::{ Request, StatusCode } };
    use http_body_util::BodyExt;
    use serde_json::json;
    use tower::ServiceExt;

    use crate::api::routes::create_router;

    async fn get_response_body(response: axum::response::Response) -> serde_json::Value {
        let body = response.into_body();
        let bytes = body.collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    // ==================== Submit Order Tests ====================

    #[tokio::test]
    async fn test_submit_limit_order_accepted() {
        let state = AppState::new();
        let app = create_router(state);

        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/orders")
            .header("Content-Type", "application/json")
            .body(
                Body::from(
                    json!({
                    "symbol": "AAPL",
                    "side": "BUY",
                    "type": "LIMIT",
                    "price": 15050,
                    "quantity": 100
                }).to_string()
                )
            )
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);

        let body = get_response_body(response).await;
        assert_eq!(body["status"], "ACCEPTED");
        assert_eq!(body["message"], "Order added to book");
        assert!(body["order_id"].is_string());
    }

    #[tokio::test]
    async fn test_submit_limit_order_filled() {
        let state = AppState::new();

        // Add a sell order first
        let sell_request = CreateOrderRequest {
            symbol: "AAPL".to_string(),
            side: Side::Sell,
            order_type: OrderType::Limit,
            price: Some(15050),
            quantity: 100,
        };
        state.engine.submit_order(sell_request).unwrap();

        let app = create_router(state);

        // Submit matching buy order
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/orders")
            .header("Content-Type", "application/json")
            .body(
                Body::from(
                    json!({
                    "symbol": "AAPL",
                    "side": "BUY",
                    "type": "LIMIT",
                    "price": 15050,
                    "quantity": 100
                }).to_string()
                )
            )
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = get_response_body(response).await;
        assert_eq!(body["status"], "FILLED");
        assert_eq!(body["filled_quantity"], 100);
        assert!(body["trades"].is_array());
        assert_eq!(body["trades"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_submit_limit_order_partial_fill() {
        let state = AppState::new();

        // Add a small sell order first
        let sell_request = CreateOrderRequest {
            symbol: "AAPL".to_string(),
            side: Side::Sell,
            order_type: OrderType::Limit,
            price: Some(15050),
            quantity: 40,
        };
        state.engine.submit_order(sell_request).unwrap();

        let app = create_router(state);

        // Submit larger buy order
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/orders")
            .header("Content-Type", "application/json")
            .body(
                Body::from(
                    json!({
                    "symbol": "AAPL",
                    "side": "BUY",
                    "type": "LIMIT",
                    "price": 15050,
                    "quantity": 100
                }).to_string()
                )
            )
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::ACCEPTED);

        let body = get_response_body(response).await;
        assert_eq!(body["status"], "PARTIAL_FILL");
        assert_eq!(body["filled_quantity"], 40);
        assert_eq!(body["remaining_quantity"], 60);
    }

    #[tokio::test]
    async fn test_submit_market_order_insufficient_liquidity() {
        let state = AppState::new();
        let app = create_router(state);

        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/orders")
            .header("Content-Type", "application/json")
            .body(
                Body::from(
                    json!({
                    "symbol": "AAPL",
                    "side": "BUY",
                    "type": "MARKET",
                    "quantity": 100
                }).to_string()
                )
            )
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = get_response_body(response).await;
        assert!(body["error"].as_str().unwrap().contains("Insufficient liquidity"));
    }

    #[tokio::test]
    async fn test_submit_invalid_order_missing_price() {
        let state = AppState::new();
        let app = create_router(state);

        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/orders")
            .header("Content-Type", "application/json")
            .body(
                Body::from(
                    json!({
                    "symbol": "AAPL",
                    "side": "BUY",
                    "type": "LIMIT",
                    "quantity": 100
                }).to_string()
                )
            )
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = get_response_body(response).await;
        assert!(body["error"].as_str().unwrap().contains("Price"));
    }

    #[tokio::test]
    async fn test_submit_invalid_order_zero_quantity() {
        let state = AppState::new();
        let app = create_router(state);

        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/orders")
            .header("Content-Type", "application/json")
            .body(
                Body::from(
                    json!({
                    "symbol": "AAPL",
                    "side": "BUY",
                    "type": "LIMIT",
                    "price": 15050,
                    "quantity": 0
                }).to_string()
                )
            )
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = get_response_body(response).await;
        assert!(body["error"].as_str().unwrap().contains("Quantity"));
    }

    // ==================== Cancel Order Tests ====================

    #[tokio::test]
    async fn test_cancel_order_success() {
        let state = AppState::new();

        // Add an order first
        let order_request = CreateOrderRequest {
            symbol: "AAPL".to_string(),
            side: Side::Buy,
            order_type: OrderType::Limit,
            price: Some(15050),
            quantity: 100,
        };
        let response = state.engine.submit_order(order_request).unwrap();
        let order_id = match response {
            SubmitOrderResponse::Accepted(r) => r.order_id,
            _ => panic!("Expected Accepted"),
        };

        let app = create_router(state);

        let request = Request::builder()
            .method("DELETE")
            .uri(format!("/api/v1/orders/{}", order_id))
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = get_response_body(response).await;
        assert_eq!(body["order_id"], order_id.to_string());
        assert_eq!(body["status"], "CANCELLED");
    }

    #[tokio::test]
    async fn test_cancel_order_not_found() {
        let state = AppState::new();
        let app = create_router(state);

        let fake_id = Uuid::new_v4();
        let request = Request::builder()
            .method("DELETE")
            .uri(format!("/api/v1/orders/{}", fake_id))
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let body = get_response_body(response).await;
        assert!(body["error"].as_str().unwrap().contains("not found"));
    }

    #[tokio::test]
    async fn test_cancel_filled_order() {
        let state = AppState::new();

        // Add sell order
        let sell_request = CreateOrderRequest {
            symbol: "AAPL".to_string(),
            side: Side::Sell,
            order_type: OrderType::Limit,
            price: Some(15050),
            quantity: 100,
        };
        let sell_response = state.engine.submit_order(sell_request).unwrap();
        let sell_order_id = match sell_response {
            SubmitOrderResponse::Accepted(r) => r.order_id,
            _ => panic!("Expected Accepted"),
        };

        // Fill it with a buy order
        let buy_request = CreateOrderRequest {
            symbol: "AAPL".to_string(),
            side: Side::Buy,
            order_type: OrderType::Limit,
            price: Some(15050),
            quantity: 100,
        };
        state.engine.submit_order(buy_request).unwrap();

        let app = create_router(state);

        // Try to cancel the filled order
        let request = Request::builder()
            .method("DELETE")
            .uri(format!("/api/v1/orders/{}", sell_order_id))
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = get_response_body(response).await;
        assert!(body["error"].as_str().unwrap().contains("already filled"));
    }

    // ==================== Get Order Status Tests ====================

    #[tokio::test]
    async fn test_get_order_status_success() {
        let state = AppState::new();

        // Add an order
        let order_request = CreateOrderRequest {
            symbol: "AAPL".to_string(),
            side: Side::Buy,
            order_type: OrderType::Limit,
            price: Some(15050),
            quantity: 100,
        };
        let response = state.engine.submit_order(order_request).unwrap();
        let order_id = match response {
            SubmitOrderResponse::Accepted(r) => r.order_id,
            _ => panic!("Expected Accepted"),
        };

        let app = create_router(state);

        let request = Request::builder()
            .method("GET")
            .uri(format!("/api/v1/orders/{}", order_id))
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = get_response_body(response).await;
        assert_eq!(body["order_id"], order_id.to_string());
        assert_eq!(body["symbol"], "AAPL");
        assert_eq!(body["side"], "BUY");
        assert_eq!(body["type"], "LIMIT");
        assert_eq!(body["price"], 15050);
        assert_eq!(body["quantity"], 100);
        assert_eq!(body["filled_quantity"], 0);
        assert_eq!(body["remaining_quantity"], 100);
        assert_eq!(body["status"], "ACCEPTED");
    }

    #[tokio::test]
    async fn test_get_order_status_not_found() {
        let state = AppState::new();
        let app = create_router(state);

        let fake_id = Uuid::new_v4();
        let request = Request::builder()
            .method("GET")
            .uri(format!("/api/v1/orders/{}", fake_id))
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    // ==================== Get Order Book Tests ====================

    #[tokio::test]
    async fn test_get_order_book() {
        let state = AppState::new();

        // Add some orders
        state.engine
            .submit_order(CreateOrderRequest {
                symbol: "AAPL".to_string(),
                side: Side::Buy,
                order_type: OrderType::Limit,
                price: Some(15000),
                quantity: 100,
            })
            .unwrap();

        state.engine
            .submit_order(CreateOrderRequest {
                symbol: "AAPL".to_string(),
                side: Side::Sell,
                order_type: OrderType::Limit,
                price: Some(15100),
                quantity: 50,
            })
            .unwrap();

        let app = create_router(state);

        let request = Request::builder()
            .method("GET")
            .uri("/api/v1/orderbook/AAPL")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = get_response_body(response).await;
        assert_eq!(body["symbol"], "AAPL");
        assert!(body["timestamp"].is_number());

        let bids = body["bids"].as_array().unwrap();
        assert_eq!(bids.len(), 1);
        assert_eq!(bids[0]["price"], 15000);
        assert_eq!(bids[0]["quantity"], 100);

        let asks = body["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1);
        assert_eq!(asks[0]["price"], 15100);
        assert_eq!(asks[0]["quantity"], 50);
    }

    #[tokio::test]
    async fn test_get_order_book_with_depth() {
        let state = AppState::new();

        // Add multiple price levels
        for i in 0..5 {
            state.engine
                .submit_order(CreateOrderRequest {
                    symbol: "AAPL".to_string(),
                    side: Side::Buy,
                    order_type: OrderType::Limit,
                    price: Some(15000 - i * 100),
                    quantity: 100,
                })
                .unwrap();
        }

        let app = create_router(state);

        let request = Request::builder()
            .method("GET")
            .uri("/api/v1/orderbook/AAPL?depth=3")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = get_response_body(response).await;
        let bids = body["bids"].as_array().unwrap();
        assert_eq!(bids.len(), 3);
    }

    #[tokio::test]
    async fn test_get_order_book_empty_symbol() {
        let state = AppState::new();
        let app = create_router(state);

        let request = Request::builder()
            .method("GET")
            .uri("/api/v1/orderbook/UNKNOWN")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = get_response_body(response).await;
        assert_eq!(body["symbol"], "UNKNOWN");
        assert!(body["bids"].as_array().unwrap().is_empty());
        assert!(body["asks"].as_array().unwrap().is_empty());
    }

    // ==================== Metrics Tests ====================

    #[tokio::test]
    async fn test_get_metrics() {
        let state = AppState::new();
        let app = create_router(state);

        let request = Request::builder().method("GET").uri("/metrics").body(Body::empty()).unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = get_response_body(response).await;
        assert!(body["orders_received"].is_number());
        assert!(body["orders_matched"].is_number());
        assert!(body["orders_cancelled"].is_number());
        assert!(body["orders_in_book"].is_number());
        assert!(body["trades_executed"].is_number());
        assert!(body["latency_p50_ms"].is_number());
        assert!(body["latency_p99_ms"].is_number());
        assert!(body["latency_p999_ms"].is_number());
        assert!(body["throughput_orders_per_sec"].is_number());
    }

    #[tokio::test]
    async fn test_get_metrics_after_activity() {
        let state = AppState::new();

        // Add some orders
        state.engine
            .submit_order(CreateOrderRequest {
                symbol: "AAPL".to_string(),
                side: Side::Sell,
                order_type: OrderType::Limit,
                price: Some(15000),
                quantity: 100,
            })
            .unwrap();

        state.engine
            .submit_order(CreateOrderRequest {
                symbol: "AAPL".to_string(),
                side: Side::Buy,
                order_type: OrderType::Limit,
                price: Some(15000),
                quantity: 100,
            })
            .unwrap();

        let app = create_router(state);

        let request = Request::builder().method("GET").uri("/metrics").body(Body::empty()).unwrap();

        let response = app.oneshot(request).await.unwrap();

        let body = get_response_body(response).await;
        assert_eq!(body["orders_received"], 2);
        assert_eq!(body["orders_matched"], 2);
        assert_eq!(body["trades_executed"], 1);
        assert_eq!(body["orders_in_book"], 0);
    }

    // ==================== Health Check Tests ====================

    #[tokio::test]
    async fn test_health_check() {
        let state = AppState::new();

        // Add some orders
        state.engine
            .submit_order(CreateOrderRequest {
                symbol: "AAPL".to_string(),
                side: Side::Buy,
                order_type: OrderType::Limit,
                price: Some(15000),
                quantity: 100,
            })
            .unwrap();

        let app = create_router(state);

        let request = Request::builder().method("GET").uri("/health").body(Body::empty()).unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = get_response_body(response).await;
        assert_eq!(body["status"], "healthy");
        assert!(body["uptime_seconds"].is_number());
        assert_eq!(body["orders_processed"], 1);
    }
}
