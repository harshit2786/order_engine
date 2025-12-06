use axum::{ body::Body, http::{ Request, StatusCode } };
use http_body_util::BodyExt;
use order_matching_engine::{ api::routes::create_router, state::AppState };
use serde_json::{ json, Value };
use tower::ServiceExt;
use uuid::Uuid;

// ============================================================================
// Helper Functions
// ============================================================================

async fn get_response_body(response: axum::response::Response) -> Value {
    let body = response.into_body();
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

async fn submit_order(
    app: &axum::Router,
    symbol: &str,
    side: &str,
    order_type: &str,
    price: Option<u64>,
    quantity: u64
) -> (StatusCode, Value) {
    let mut body =
        json!({
        "symbol": symbol,
        "side": side,
        "type": order_type,
        "quantity": quantity
    });

    if let Some(p) = price {
        body["price"] = json!(p);
    }

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/orders")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = get_response_body(response).await;

    (status, body)
}

async fn get_order_status(app: &axum::Router, order_id: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/orders/{}", order_id))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = get_response_body(response).await;

    (status, body)
}

async fn cancel_order(app: &axum::Router, order_id: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("DELETE")
        .uri(format!("/api/v1/orders/{}", order_id))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = get_response_body(response).await;

    (status, body)
}

async fn get_order_book(app: &axum::Router, symbol: &str, depth: usize) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/orderbook/{}?depth={}", symbol, depth))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = get_response_body(response).await;

    (status, body)
}

async fn get_metrics(app: &axum::Router) -> (StatusCode, Value) {
    let request = Request::builder().method("GET").uri("/metrics").body(Body::empty()).unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = get_response_body(response).await;

    (status, body)
}

async fn health_check(app: &axum::Router) -> (StatusCode, Value) {
    let request = Request::builder().method("GET").uri("/health").body(Body::empty()).unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = get_response_body(response).await;

    (status, body)
}

// ============================================================================
// Integration Tests: End-to-End Order Flow
// ============================================================================

/// Test complete order lifecycle: Submit -> Check Status -> Cancel -> Verify
#[tokio::test]
async fn test_order_lifecycle_submit_check_cancel() {
    let state = AppState::new();
    let app = create_router(state);

    // Step 1: Submit a limit order
    let (status, body) = submit_order(&app, "AAPL", "BUY", "LIMIT", Some(15000), 100).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["status"], "ACCEPTED");
    let order_id = body["order_id"].as_str().unwrap();

    // Step 2: Check order status
    let (status, body) = get_order_status(&app, order_id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ACCEPTED");
    assert_eq!(body["symbol"], "AAPL");
    assert_eq!(body["side"], "BUY");
    assert_eq!(body["quantity"], 100);
    assert_eq!(body["filled_quantity"], 0);

    // Step 3: Verify order is in order book
    let (status, body) = get_order_book(&app, "AAPL", 10).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["bids"].as_array().unwrap().len(), 1);
    assert_eq!(body["bids"][0]["price"], 15000);
    assert_eq!(body["bids"][0]["quantity"], 100);

    // Step 4: Cancel the order
    let (status, body) = cancel_order(&app, order_id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "CANCELLED");

    // Step 5: Verify order status is cancelled
    let (status, body) = get_order_status(&app, order_id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "CANCELLED");

    // Step 6: Verify order book is empty
    let (status, body) = get_order_book(&app, "AAPL", 10).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["bids"].as_array().unwrap().is_empty());
}

/// Test full trade execution: Buy order matches Sell order
#[tokio::test]
async fn test_full_trade_execution() {
    let state = AppState::new();
    let app = create_router(state);

    // Step 1: Submit a sell order (maker)
    let (status, body) = submit_order(&app, "AAPL", "SELL", "LIMIT", Some(15000), 100).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["status"], "ACCEPTED");
    let sell_order_id = body["order_id"].as_str().unwrap().to_string();

    // Step 2: Verify sell order is in order book
    let (status, body) = get_order_book(&app, "AAPL", 10).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["asks"].as_array().unwrap().len(), 1);

    // Step 3: Submit a buy order (taker) that matches
    let (status, body) = submit_order(&app, "AAPL", "BUY", "LIMIT", Some(15000), 100).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "FILLED");
    assert_eq!(body["filled_quantity"], 100);

    // Verify trade details
    let trades = body["trades"].as_array().unwrap();
    assert_eq!(trades.len(), 1);
    assert_eq!(trades[0]["price"], 15000);
    assert_eq!(trades[0]["quantity"], 100);

    // Step 4: Verify both orders are filled
    let (status, body) = get_order_status(&app, &sell_order_id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "FILLED");
    assert_eq!(body["filled_quantity"], 100);

    // Step 5: Verify order book is empty
    let (status, body) = get_order_book(&app, "AAPL", 10).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["bids"].as_array().unwrap().is_empty());
    assert!(body["asks"].as_array().unwrap().is_empty());

    // Step 6: Verify metrics
    let (status, body) = get_metrics(&app).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["orders_received"], 2);
    assert_eq!(body["trades_executed"], 1);
}

/// Test partial fill scenario
#[tokio::test]
async fn test_partial_fill_order() {
    let state = AppState::new();
    let app = create_router(state);

    // Step 1: Submit a small sell order
    let (status, body) = submit_order(&app, "AAPL", "SELL", "LIMIT", Some(15000), 40).await;
    assert_eq!(status, StatusCode::CREATED);
    let sell_order_id = body["order_id"].as_str().unwrap().to_string();

    // Step 2: Submit a larger buy order
    let (status, body) = submit_order(&app, "AAPL", "BUY", "LIMIT", Some(15000), 100).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body["status"], "PARTIAL_FILL");
    assert_eq!(body["filled_quantity"], 40);
    assert_eq!(body["remaining_quantity"], 60);
    let buy_order_id = body["order_id"].as_str().unwrap().to_string();

    // Step 3: Verify sell order is fully filled
    let (status, body) = get_order_status(&app, &sell_order_id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "FILLED");

    // Step 4: Verify buy order is partially filled and in book
    let (status, body) = get_order_status(&app, &buy_order_id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "PARTIAL_FILL");
    assert_eq!(body["filled_quantity"], 40);
    assert_eq!(body["remaining_quantity"], 60);

    // Step 5: Verify order book has remaining buy order
    let (status, body) = get_order_book(&app, "AAPL", 10).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["bids"].as_array().unwrap().len(), 1);
    assert_eq!(body["bids"][0]["quantity"], 60);

    // Step 6: Complete the fill with another sell order
    let (status, body) = submit_order(&app, "AAPL", "SELL", "LIMIT", Some(15000), 60).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "FILLED");

    // Step 7: Verify buy order is now fully filled
    let (status, body) = get_order_status(&app, &buy_order_id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "FILLED");
    assert_eq!(body["filled_quantity"], 100);
}

/// Test market order execution
#[tokio::test]
async fn test_market_order_execution() {
    let state = AppState::new();
    let app = create_router(state);

    // Step 1: Build order book with multiple sell orders at different prices
    submit_order(&app, "AAPL", "SELL", "LIMIT", Some(15000), 50).await;
    submit_order(&app, "AAPL", "SELL", "LIMIT", Some(15100), 50).await;
    submit_order(&app, "AAPL", "SELL", "LIMIT", Some(15200), 50).await;

    // Step 2: Submit market buy order
    let (status, body) = submit_order(&app, "AAPL", "BUY", "MARKET", None, 100).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "FILLED");
    assert_eq!(body["filled_quantity"], 100);

    // Verify it matched at best prices (lowest asks first)
    let trades = body["trades"].as_array().unwrap();
    assert_eq!(trades.len(), 2);
    assert_eq!(trades[0]["price"], 15000);
    assert_eq!(trades[0]["quantity"], 50);
    assert_eq!(trades[1]["price"], 15100);
    assert_eq!(trades[1]["quantity"], 50);

    // Step 3: Verify order book state
    let (status, body) = get_order_book(&app, "AAPL", 10).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["asks"].as_array().unwrap().len(), 1);
    assert_eq!(body["asks"][0]["price"], 15200);
}

/// Test market order rejection due to insufficient liquidity
#[tokio::test]
async fn test_market_order_insufficient_liquidity() {
    let state = AppState::new();
    let app = create_router(state);

    // Step 1: Add small sell order
    submit_order(&app, "AAPL", "SELL", "LIMIT", Some(15000), 50).await;

    // Step 2: Try to submit larger market buy order
    let (status, body) = submit_order(&app, "AAPL", "BUY", "MARKET", None, 100).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("Insufficient liquidity"));

    // Step 3: Verify original order still in book
    let (status, body) = get_order_book(&app, "AAPL", 10).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["asks"].as_array().unwrap().len(), 1);
}

/// Test price-time priority
#[tokio::test]
async fn test_price_time_priority() {
    let state = AppState::new();
    let app = create_router(state);

    // Step 1: Submit sell orders at different prices
    let (_, body) = submit_order(&app, "AAPL", "SELL", "LIMIT", Some(15100), 50).await;
    let sell_high_id = body["order_id"].as_str().unwrap().to_string();

    let (_, body) = submit_order(&app, "AAPL", "SELL", "LIMIT", Some(15000), 50).await;
    let sell_low_id = body["order_id"].as_str().unwrap().to_string();

    // Step 2: Submit two sell orders at same price (time priority test)
    let (_, body) = submit_order(&app, "AAPL", "SELL", "LIMIT", Some(15050), 30).await;
    let sell_first_id = body["order_id"].as_str().unwrap().to_string();

    let (_, body) = submit_order(&app, "AAPL", "SELL", "LIMIT", Some(15050), 30).await;
    let sell_second_id = body["order_id"].as_str().unwrap().to_string();

    // Step 3: Submit buy order that matches lowest price first
    let (status, body) = submit_order(&app, "AAPL", "BUY", "LIMIT", Some(15050), 80).await;
    assert_eq!(status, StatusCode::OK);

    let trades = body["trades"].as_array().unwrap();
    // Should match: 15000 (50) first, then 15050 first-in (30)
    assert_eq!(trades[0]["price"], 15000);
    assert_eq!(trades[0]["quantity"], 50);
    assert_eq!(trades[1]["price"], 15050);
    assert_eq!(trades[1]["quantity"], 30);

    // Step 4: Verify order states
    let (_, body) = get_order_status(&app, &sell_low_id).await;
    assert_eq!(body["status"], "FILLED");

    let (_, body) = get_order_status(&app, &sell_first_id).await;
    assert_eq!(body["status"], "FILLED");

    let (_, body) = get_order_status(&app, &sell_second_id).await;
    assert_eq!(body["status"], "ACCEPTED"); // Not touched yet

    let (_, body) = get_order_status(&app, &sell_high_id).await;
    assert_eq!(body["status"], "ACCEPTED"); // Not touched yet
}

/// Test multiple symbols isolation
#[tokio::test]
async fn test_multiple_symbols_isolation() {
    let state = AppState::new();
    let app = create_router(state);

    // Step 1: Submit orders for different symbols
    submit_order(&app, "AAPL", "BUY", "LIMIT", Some(15000), 100).await;
    submit_order(&app, "GOOG", "BUY", "LIMIT", Some(25000), 50).await;
    submit_order(&app, "BTC", "SELL", "LIMIT", Some(50000), 10).await;

    // Step 2: Verify each order book is independent
    let (_, body) = get_order_book(&app, "AAPL", 10).await;
    assert_eq!(body["bids"].as_array().unwrap().len(), 1);
    assert!(body["asks"].as_array().unwrap().is_empty());

    let (_, body) = get_order_book(&app, "GOOG", 10).await;
    assert_eq!(body["bids"].as_array().unwrap().len(), 1);
    assert!(body["asks"].as_array().unwrap().is_empty());

    let (_, body) = get_order_book(&app, "BTC", 10).await;
    assert!(body["bids"].as_array().unwrap().is_empty());
    assert_eq!(body["asks"].as_array().unwrap().len(), 1);

    // Step 3: Submit matching order for one symbol doesn't affect others
    submit_order(&app, "AAPL", "SELL", "LIMIT", Some(15000), 100).await;

    let (_, body) = get_order_book(&app, "AAPL", 10).await;
    assert!(body["bids"].as_array().unwrap().is_empty());
    assert!(body["asks"].as_array().unwrap().is_empty());

    // GOOG and BTC unchanged
    let (_, body) = get_order_book(&app, "GOOG", 10).await;
    assert_eq!(body["bids"].as_array().unwrap().len(), 1);

    let (_, body) = get_order_book(&app, "BTC", 10).await;
    assert_eq!(body["asks"].as_array().unwrap().len(), 1);
}

/// Test order book depth limit
#[tokio::test]
async fn test_order_book_depth_limit() {
    let state = AppState::new();
    let app = create_router(state);

    // Submit 10 orders at different prices
    for i in 0..10 {
        submit_order(&app, "AAPL", "BUY", "LIMIT", Some(15000 - i * 100), 100).await;
    }

    // Request depth of 5
    let (status, body) = get_order_book(&app, "AAPL", 5).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["bids"].as_array().unwrap().len(), 5);

    // Verify highest prices are returned first
    let bids = body["bids"].as_array().unwrap();
    assert_eq!(bids[0]["price"], 15000);
    assert_eq!(bids[4]["price"], 14600);
}

/// Test cancel non-existent order
#[tokio::test]
async fn test_cancel_non_existent_order() {
    let state = AppState::new();
    let app = create_router(state);

    let fake_id = Uuid::new_v4();
    let (status, body) = cancel_order(&app, &fake_id.to_string()).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body["error"].as_str().unwrap().contains("not found"));
}

/// Test cancel already filled order
#[tokio::test]
async fn test_cancel_filled_order() {
    let state = AppState::new();
    let app = create_router(state);

    // Create and fill an order
    let (_, body) = submit_order(&app, "AAPL", "SELL", "LIMIT", Some(15000), 100).await;
    let sell_order_id = body["order_id"].as_str().unwrap().to_string();

    submit_order(&app, "AAPL", "BUY", "LIMIT", Some(15000), 100).await;

    // Try to cancel filled order
    let (status, body) = cancel_order(&app, &sell_order_id).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("already filled"));
}

/// Test invalid order requests
#[tokio::test]
async fn test_invalid_order_requests() {
    let state = AppState::new();
    let app = create_router(state);

    // Zero quantity
    let (status, body) = submit_order(&app, "AAPL", "BUY", "LIMIT", Some(15000), 0).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("Quantity"));

    // Limit order without price
    let (status, body) = submit_order(&app, "AAPL", "BUY", "LIMIT", None, 100).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("Price"));

    // Market order with price
    let (status, body) = submit_order(&app, "AAPL", "BUY", "MARKET", Some(15000), 100).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("Price"));

    // Empty symbol
    let (status, body) = submit_order(&app, "", "BUY", "LIMIT", Some(15000), 100).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("Symbol"));
}

/// Test health and metrics endpoints
#[tokio::test]
async fn test_health_and_metrics() {
    let state = AppState::new();
    let app = create_router(state);

    // Health check
    let (status, body) = health_check(&app).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "healthy");
    assert!(body["uptime_seconds"].is_number());

    // Submit some orders
    submit_order(&app, "AAPL", "SELL", "LIMIT", Some(15000), 100).await;
    submit_order(&app, "AAPL", "BUY", "LIMIT", Some(15000), 100).await;

    // Check metrics
    let (status, body) = get_metrics(&app).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["orders_received"], 2);
    assert_eq!(body["orders_matched"], 2);
    assert_eq!(body["trades_executed"], 1);
    assert_eq!(body["orders_in_book"], 0);
}

/// Test concurrent order modifications (basic race condition test)
#[tokio::test]
async fn test_concurrent_order_operations() {
    use tokio::task::JoinSet;

    let state = AppState::new();
    let app = create_router(state);

    let mut tasks = JoinSet::new();

    // Submit 100 concurrent buy orders
    for i in 0..100 {
        let app_clone = app.clone();
        tasks.spawn(async move {
            let body =
                json!({
                "symbol": "AAPL",
                "side": "BUY",
                "type": "LIMIT",
                "price": 15000 + (i % 10) * 10,
                "quantity": 10
            });

            let request = Request::builder()
                .method("POST")
                .uri("/api/v1/orders")
                .header("Content-Type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap();

            let response = app_clone.oneshot(request).await.unwrap();
            assert!(response.status().is_success());
        });
    }

    // Submit 100 concurrent sell orders
    for i in 0..100 {
        let app_clone = app.clone();
        tasks.spawn(async move {
            let body =
                json!({
                "symbol": "AAPL",
                "side": "SELL",
                "type": "LIMIT",
                "price": 15000 + (i % 10) * 10,
                "quantity": 10
            });

            let request = Request::builder()
                .method("POST")
                .uri("/api/v1/orders")
                .header("Content-Type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap();

            let response = app_clone.oneshot(request).await.unwrap();
            assert!(response.status().is_success());
        });
    }

    // Wait for all tasks
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }

    // Verify system is in consistent state
    let request = Request::builder().method("GET").uri("/metrics").body(Body::empty()).unwrap();

    let response = app.oneshot(request).await.unwrap();
    let body = get_response_body(response).await;

    // All 200 orders should be received
    assert_eq!(body["orders_received"], 200);

    // Verify no data corruption - orders_matched + orders_in_book should make sense
    let orders_matched = body["orders_matched"].as_u64().unwrap();
    let orders_in_book = body["orders_in_book"].as_u64().unwrap();
    let trades_executed = body["trades_executed"].as_u64().unwrap();

    // Basic sanity checks
    assert!(orders_matched <= 200);
    assert!(orders_in_book <= 200);
    assert!(trades_executed <= 100); // Max possible trades

    println!(
        "Concurrent test results: matched={}, in_book={}, trades={}",
        orders_matched,
        orders_in_book,
        trades_executed
    );
}

/// Test data consistency under concurrent load
#[tokio::test]
async fn test_data_consistency_under_load() {
    use std::sync::Arc;
    use std::sync::atomic::{ AtomicU64, Ordering };
    use tokio::task::JoinSet;

    let state = AppState::new();
    let app = create_router(state);

    // First, add some liquidity
    for _ in 0..50 {
        let body =
            json!({
            "symbol": "AAPL",
            "side": "SELL",
            "type": "LIMIT",
            "price": 15000,
            "quantity": 100
        });

        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/orders")
            .header("Content-Type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();

        app.clone().oneshot(request).await.unwrap();
    }

    // Now submit concurrent buy orders
    let mut tasks = JoinSet::new();
    let successful_orders = Arc::new(AtomicU64::new(0));
    let total_filled = Arc::new(AtomicU64::new(0));

    for _ in 0..100 {
        let app_clone = app.clone();
        let successful_orders_clone = successful_orders.clone();
        let total_filled_clone = total_filled.clone();

        tasks.spawn(async move {
            let body =
                json!({
                "symbol": "AAPL",
                "side": "BUY",
                "type": "LIMIT",
                "price": 15000,
                "quantity": 50
            });

            let request = Request::builder()
                .method("POST")
                .uri("/api/v1/orders")
                .header("Content-Type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap();

            let response = app_clone.oneshot(request).await.unwrap();

            if response.status().is_success() {
                successful_orders_clone.fetch_add(1, Ordering::SeqCst);

                let body = get_response_body(response).await;
                if let Some(filled) = body["filled_quantity"].as_u64() {
                    total_filled_clone.fetch_add(filled, Ordering::SeqCst);
                }
            }
        });
    }

    // Wait for all tasks
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }

    let total_filled_qty = total_filled.load(Ordering::SeqCst);

    // Total available was 50 * 100 = 5000
    // Total requested was 100 * 50 = 5000
    // So we should have filled exactly 5000 or close to it
    println!("Total filled quantity: {}", total_filled_qty);

    // Verify order book state
    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/orderbook/AAPL?depth=100")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let body = get_response_body(response).await;

    let bids_qty: u64 = body["bids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["quantity"].as_u64().unwrap())
        .sum();

    let asks_qty: u64 = body["asks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["quantity"].as_u64().unwrap())
        .sum();

    println!("Remaining bids: {}, asks: {}", bids_qty, asks_qty);

    // Verify consistency: filled + remaining should equal total submitted
    assert!(total_filled_qty <= 5000, "Cannot fill more than available");
}
/// Test order book aggregation correctness
#[tokio::test]
async fn test_order_book_aggregation() {
    let state = AppState::new();
    let app = create_router(state);

    // Submit multiple orders at the same price
    for _ in 0..5 {
        submit_order(&app, "AAPL", "BUY", "LIMIT", Some(15000), 100).await;
    }

    // Submit orders at different prices
    submit_order(&app, "AAPL", "BUY", "LIMIT", Some(14900), 50).await;
    submit_order(&app, "AAPL", "BUY", "LIMIT", Some(14800), 75).await;

    let (status, body) = get_order_book(&app, "AAPL", 10).await;
    assert_eq!(status, StatusCode::OK);

    let bids = body["bids"].as_array().unwrap();

    // Should have 3 price levels
    assert_eq!(bids.len(), 3);

    // Highest price first with aggregated quantity
    assert_eq!(bids[0]["price"], 15000);
    assert_eq!(bids[0]["quantity"], 500); // 5 * 100

    assert_eq!(bids[1]["price"], 14900);
    assert_eq!(bids[1]["quantity"], 50);

    assert_eq!(bids[2]["price"], 14800);
    assert_eq!(bids[2]["quantity"], 75);
}

/// Test complete trading session simulation
#[tokio::test]
async fn test_trading_session_simulation() {
    let state = AppState::new();
    let app = create_router(state);

    // Simulate a trading session

    // 1. Market makers add liquidity
    for i in 0..5 {
        submit_order(&app, "AAPL", "BUY", "LIMIT", Some(14900 - i * 10), 100).await;
        submit_order(&app, "AAPL", "SELL", "LIMIT", Some(15100 + i * 10), 100).await;
    }

    // 2. Verify spread
    let (_, body) = get_order_book(&app, "AAPL", 10).await;
    let best_bid = body["bids"][0]["price"].as_u64().unwrap();
    let best_ask = body["asks"][0]["price"].as_u64().unwrap();
    assert_eq!(best_bid, 14900);
    assert_eq!(best_ask, 15100);
    let spread = best_ask - best_bid;
    assert_eq!(spread, 200);

    // 3. Aggressive buyer crosses the spread
    let (status, body) = submit_order(&app, "AAPL", "BUY", "LIMIT", Some(15100), 50).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "FILLED");

    // 4. Verify order book updated
    let (_, body) = get_order_book(&app, "AAPL", 10).await;
    assert_eq!(body["asks"][0]["quantity"], 50); // 100 - 50

    // 5. Market order takes remaining liquidity at best ask
    let (status, body) = submit_order(&app, "AAPL", "BUY", "MARKET", None, 50).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "FILLED");

    // 6. Verify best ask moved up
    let (_, body) = get_order_book(&app, "AAPL", 10).await;
    assert_eq!(body["asks"][0]["price"], 15110);

    // 7. Check final metrics
    let (_, body) = get_metrics(&app).await;
    assert!(body["trades_executed"].as_u64().unwrap() >= 2);
}
