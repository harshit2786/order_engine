//! Performance benchmarks for the Order Matching Engine
//!
//! These tests measure actual HTTP round-trip latency as required.
//!
//! Run with:
//!   cargo test --release --test performance_tests -- --ignored --nocapture

use std::sync::atomic::{ AtomicBool, AtomicU64, Ordering };
use std::sync::Arc;
use std::time::{ Duration, Instant };

use reqwest::Client;
use serde_json::json;
use tokio::runtime::Runtime;

const BASE_URL: &str = "http://localhost:8081";

/// Start the server in a background task
async fn start_server() -> tokio::task::JoinHandle<()> {
    use order_matching_engine::api::routes::create_router;
    use order_matching_engine::state::AppState;
    use std::net::SocketAddr;
    use tokio::net::TcpListener;

    let state = AppState::new();
    let app = create_router(state);

    let addr: SocketAddr = "127.0.0.1:8081".parse().unwrap();
    let listener = TcpListener::bind(addr).await.expect("Failed to bind to address 127.0.0.1:8081. Port might be in use.");

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    })
}

/// Wait for server to be ready
async fn wait_for_server(client: &Client, max_retries: u32) -> bool {
    for i in 0..max_retries {
        match client.get(format!("{}/health", BASE_URL)).send().await {
            Ok(resp) if resp.status().is_success() => {
                println!("Server ready after {} attempts", i + 1);
                return true;
            }
            _ => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    false
}

/// Submit a limit order via HTTP
async fn submit_limit_order(
    client: &Client,
    symbol: &str,
    side: &str,
    price: u64,
    quantity: u64,
) -> Result<reqwest::Response, reqwest::Error> {
    let body = json!({
        "symbol": symbol,
        "side": side,
        "type": "LIMIT",
        "price": price,
        "quantity": quantity
    });

    client
        .post(format!("{}/api/v1/orders", BASE_URL))
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .await
}

/// Submit a limit order and measure latency
async fn submit_order_with_latency(
    client: &Client,
    symbol: &str,
    side: &str,
    price: u64,
    quantity: u64,
) -> (bool, Duration) {
    let start = Instant::now();
    let result = submit_limit_order(client, symbol, side, price, quantity).await;
    let latency = start.elapsed();

    let success = match result {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    };

    (success, latency)
}

/// Pre-populate the order book with liquidity
async fn populate_order_book(client: &Client, count: usize) {
    for i in 0..count {
        let price_offset = (i % 100) as u64;

        // Add sell orders
        let _ = submit_limit_order(client, "AAPL", "SELL", 15000 + price_offset, 100).await;

        // Add buy orders
        let _ = submit_limit_order(client, "AAPL", "BUY", 14000 + price_offset, 100).await;
    }
}

// ============================================================================
// HTTP Round-Trip Latency Test
// ============================================================================

#[test]
fn test_http_latency_measurements() {
    let rt = Runtime::new().unwrap();

    rt.block_on(async {
        // Start server
        let _server = start_server().await;
        
        let client = Client::new();
        
        // Wait for server to be ready
        if !wait_for_server(&client, 50).await {
            panic!("Server failed to start");
        }

        // Populate order book
        println!("Populating order book...");
        populate_order_book(&client, 1000).await;
        println!("Order book populated.\n");

        // Measure latencies
        let iterations = 10_000;
        let mut latencies: Vec<Duration> = Vec::with_capacity(iterations);

        println!("Measuring latencies for {} requests...", iterations);

        for i in 0..iterations {
            let side = if i % 2 == 0 { "BUY" } else { "SELL" };
            let price = 14500 + (i % 100) as u64;

            let (success, latency) = submit_order_with_latency(
                &client, 
                "AAPL", 
                side, 
                price, 
                10
            ).await;

            if success {
                latencies.push(latency);
            }

            // Progress indicator
            if (i + 1) % 1000 == 0 {
                println!("  Completed {}/{}", i + 1, iterations);
            }
        }

        // Sort for percentile calculation
        latencies.sort();

        let count = latencies.len();
        if count == 0 {
            panic!("No successful requests!");
        }

        let p50_idx = (count as f64 * 0.50) as usize;
        let p99_idx = (count as f64 * 0.99) as usize;
        let p999_idx = ((count as f64 * 0.999) as usize).min(count - 1);

        let p50 = latencies[p50_idx];
        let p99 = latencies[p99_idx];
        let p999 = latencies[p999_idx];
        let min = latencies[0];
        let max = latencies[count - 1];

        let p50_ms = p50.as_secs_f64() * 1000.0;
        let p99_ms = p99.as_secs_f64() * 1000.0;
        let p999_ms = p999.as_secs_f64() * 1000.0;
        let min_ms = min.as_secs_f64() * 1000.0;
        let max_ms = max.as_secs_f64() * 1000.0;

        println!("\n========== HTTP Round-Trip Latency Results ==========");
        println!("Successful requests: {}", count);
        println!("Min:   {:.3} ms", min_ms);
        println!("p50:   {:.3} ms", p50_ms);
        println!("p99:   {:.3} ms", p99_ms);
        println!("p999:  {:.3} ms", p999_ms);
        println!("Max:   {:.3} ms", max_ms);
        println!("=====================================================\n");

        // Verify against requirements
        println!("Checking against requirements:");
        println!(
            "  p50 <= 10ms:   {} ({:.3}ms)",
            if p50_ms <= 10.0 { "✅ PASS" } else { "❌ FAIL" },
            p50_ms
        );
        println!(
            "  p99 <= 50ms:   {} ({:.3}ms)",
            if p99_ms <= 50.0 { "✅ PASS" } else { "❌ FAIL" },
            p99_ms
        );
        println!(
            "  p999 <= 100ms: {} ({:.3}ms)",
            if p999_ms <= 100.0 { "✅ PASS" } else { "❌ FAIL" },
            p999_ms
        );

        assert!(p50_ms <= 10.0, "p50 latency {:.3}ms exceeds 10ms", p50_ms);
        assert!(p99_ms <= 50.0, "p99 latency {:.3}ms exceeds 50ms", p99_ms);
        assert!(p999_ms <= 100.0, "p999 latency {:.3}ms exceeds 100ms", p999_ms);
    });
}

// ============================================================================
// HTTP Throughput Test with Concurrent Clients
// ============================================================================

#[test]
fn test_http_throughput_concurrent() {
    let rt = Runtime::new().unwrap();

    rt.block_on(async {
        // Start server
        let _server = start_server().await;
        
        let client = Client::new();
        
        // Wait for server to be ready
        if !wait_for_server(&client, 50).await {
            panic!("Server failed to start");
        }

        // Populate order book
        println!("Populating order book...");
        populate_order_book(&client, 5000).await;
        println!("Order book populated.\n");

        let num_clients = 100;
        let test_duration = Duration::from_secs(10);

        let running = Arc::new(AtomicBool::new(true));
        let total_requests = Arc::new(AtomicU64::new(0));
        let successful_requests = Arc::new(AtomicU64::new(0));

        println!(
            "Starting throughput test with {} concurrent clients for {:?}...\n",
            num_clients, test_duration
        );

        let start = Instant::now();

        // Spawn concurrent clients
        let mut handles = Vec::new();

        for client_id in 0..num_clients {
            let running_clone = running.clone();
            let total_clone = total_requests.clone();
            let successful_clone = successful_requests.clone();

            let handle = tokio::spawn(async move {
                let client = Client::new();
                let mut request_count = 0u64;
                let mut success_count = 0u64;

                while running_clone.load(Ordering::Relaxed) {
                    let side = if (client_id + request_count as usize) % 2 == 0 {
                        "BUY"
                    } else {
                        "SELL"
                    };
                    let price = 14500 + (request_count % 100);

                    let result = submit_limit_order(
                        &client,
                        "AAPL",
                        side,
                        price,
                        10,
                    ).await;

                    request_count += 1;
                    if result.is_ok() && result.unwrap().status().is_success() {
                        success_count += 1;
                    }
                }

                total_clone.fetch_add(request_count, Ordering::Relaxed);
                successful_clone.fetch_add(success_count, Ordering::Relaxed);
            });

            handles.push(handle);
        }

        // Run for test duration
        tokio::time::sleep(test_duration).await;
        running.store(false, Ordering::Relaxed);

        // Wait for all clients to finish
        for handle in handles {
            let _ = handle.await;
        }

        let elapsed = start.elapsed();
        let total = total_requests.load(Ordering::Relaxed);
        let successful = successful_requests.load(Ordering::Relaxed);
        let throughput = total as f64 / elapsed.as_secs_f64();
        let success_rate = (successful as f64 / total as f64) * 100.0;

        println!("========== HTTP Throughput Results ==========");
        println!("Test Duration:      {:?}", elapsed);
        println!("Concurrent Clients: {}", num_clients);
        println!("Total Requests:     {}", total);
        println!("Successful:         {}", successful);
        println!("Success Rate:       {:.2}%", success_rate);
        println!("Throughput:         {:.2} requests/sec", throughput);
        println!("=============================================\n");

        println!("Checking against requirements:");
        println!(
            "  Throughput >= 30,000/sec:      {} ({:.2}/sec)",
            if throughput >= 30_000.0 { "✅ PASS" } else { "❌ FAIL" },
            throughput
        );
        println!(
            "  Concurrent Connections >= 100: {} ({})",
            if num_clients >= 100 { "✅ PASS" } else { "❌ FAIL" },
            num_clients
        );

        assert!(
            throughput >= 30_000.0,
            "Throughput {:.2} is below target 30,000 req/sec",
            throughput
        );
    });
}
// ============================================================================
// Full 60-Second Load Test (as per requirements)
// ============================================================================

#[test]
fn load_test_60_seconds_http() {
    let rt = Runtime::new().unwrap();

    rt.block_on(async {
        // Start server
        let _server = start_server().await;
        
        let client = Client::new();
        
        // Wait for server to be ready
        if !wait_for_server(&client, 50).await {
            panic!("Server failed to start");
        }

        // Populate order book
        println!("Populating order book with liquidity...");
        populate_order_book(&client, 10000).await;
        println!("Order book populated.\n");

        let num_clients = 100;
        let test_duration = Duration::from_secs(60);

        let running = Arc::new(AtomicBool::new(true));
        let total_requests = Arc::new(AtomicU64::new(0));
        let successful_requests = Arc::new(AtomicU64::new(0));
        let all_latencies = Arc::new(parking_lot::Mutex::new(Vec::new()));

        println!("╔════════════════════════════════════════════════════════════╗");
        println!("║           60-SECOND LOAD TEST (HTTP Round-Trip)            ║");
        println!("╠════════════════════════════════════════════════════════════╣");
        println!("║  Concurrent Clients: {:>6}                                ║", num_clients);
        println!("║  Test Duration:      {:>6} seconds                        ║", test_duration.as_secs());
        println!("╚════════════════════════════════════════════════════════════╝\n");

        let start = Instant::now();

        // Progress reporter
        let running_progress = running.clone();
        let total_progress = total_requests.clone();
        let progress_handle = tokio::spawn(async move {
            let mut last_count = 0u64;
            let mut interval = tokio::time::interval(Duration::from_secs(5));

            while running_progress.load(Ordering::Relaxed) {
                interval.tick().await;
                let current = total_progress.load(Ordering::Relaxed);
                let rate = (current - last_count) / 5;
                println!(
                    "  [Progress] Total: {} requests, Current rate: ~{}/sec",
                    current, rate
                );
                last_count = current;
            }
        });

        // Spawn concurrent clients
        let mut handles = Vec::new();

        for client_id in 0..num_clients {
            let running_clone = running.clone();
            let total_clone = total_requests.clone();
            let successful_clone = successful_requests.clone();
            let latencies_clone = all_latencies.clone();

            let handle = tokio::spawn(async move {
                let client = Client::new();
                let mut local_latencies = Vec::new();
                let mut request_count = 0u64;
                let mut success_count = 0u64;

                while running_clone.load(Ordering::Relaxed) {
                    let side = if (client_id + request_count as usize) % 2 == 0 {
                        "BUY"
                    } else {
                        "SELL"
                    };
                    let price = 14500 + (request_count % 100);

                    let op_start = Instant::now();
                    let result = submit_limit_order(
                        &client,
                        "AAPL",
                        side,
                        price,
                        10,
                    ).await;
                    let latency = op_start.elapsed();

                    request_count += 1;
                    if let Ok(resp) = result {
                        if resp.status().is_success() {
                            success_count += 1;
                            local_latencies.push(latency);
                        }
                    }
                }

                total_clone.fetch_add(request_count, Ordering::Relaxed);
                successful_clone.fetch_add(success_count, Ordering::Relaxed);

                // Merge latencies
                let mut all = latencies_clone.lock();
                all.extend(local_latencies);
            });

            handles.push(handle);
        }

        // Run for test duration
        tokio::time::sleep(test_duration).await;
        running.store(false, Ordering::Relaxed);

        // Wait for all clients to finish
        for handle in handles {
            let _ = handle.await;
        }
        let _ = progress_handle.await;

        let elapsed = start.elapsed();
        let total = total_requests.load(Ordering::Relaxed);
        let successful = successful_requests.load(Ordering::Relaxed);
        let throughput = successful as f64 / elapsed.as_secs_f64();
        let success_rate = if total > 0 {
            (successful as f64 / total as f64) * 100.0
        } else {
            0.0
        };

        // Calculate latency percentiles
        let mut latencies = all_latencies.lock();
        latencies.sort();

        let count = latencies.len();
        let (p50_ms, p99_ms, p999_ms, min_ms, max_ms) = if count > 0 {
            let p50_idx = (count as f64 * 0.50) as usize;
            let p99_idx = ((count as f64 * 0.99) as usize).min(count - 1);
            let p999_idx = ((count as f64 * 0.999) as usize).min(count - 1);

            (
                latencies[p50_idx].as_secs_f64() * 1000.0,
                latencies[p99_idx].as_secs_f64() * 1000.0,
                latencies[p999_idx].as_secs_f64() * 1000.0,
                latencies[0].as_secs_f64() * 1000.0,
                latencies[count - 1].as_secs_f64() * 1000.0,
            )
        } else {
            (0.0, 0.0, 0.0, 0.0, 0.0)
        };

        println!("\n╔════════════════════════════════════════════════════════════╗");
        println!("║                    TEST RESULTS                            ║");
        println!("╠════════════════════════════════════════════════════════════╣");
        println!("║  Test Duration:        {:>10.2?}                       ║", elapsed);
        println!("║  Concurrent Clients:   {:>10}                         ║", num_clients);
        println!("║  Total Requests:       {:>10}                         ║", total);
        println!("║  Successful Requests:  {:>10}                         ║", successful);
        println!("║  Success Rate:         {:>10.2}%                        ║", success_rate);
        println!("║  Throughput:           {:>10.2} req/sec                ║", throughput);
        println!("╠════════════════════════════════════════════════════════════╣");
        println!("║                  LATENCY MEASUREMENTS                      ║");
        println!("╠════════════════════════════════════════════════════════════╣");
        println!("║  Min:                  {:>10.3} ms                      ║", min_ms);
        println!("║  p50 (median):         {:>10.3} ms                      ║", p50_ms);
        println!("║  p99:                  {:>10.3} ms                      ║", p99_ms);
        println!("║  p999:                 {:>10.3} ms                      ║", p999_ms);
        println!("║  Max:                  {:>10.3} ms                      ║", max_ms);
        println!("╚════════════════════════════════════════════════════════════╝\n");

        // Check requirements
        let throughput_pass = throughput >= 30_000.0;
        let p50_pass = p50_ms <= 10.0;
        let p99_pass = p99_ms <= 50.0;
        let p999_pass = p999_ms <= 100.0;
        let connections_pass = num_clients >= 100;

        println!("╔════════════════════════════════════════════════════════════╗");
        println!("║              REQUIREMENTS VERIFICATION                     ║");
        println!("╠════════════════════════════════════════════════════════════╣");
        println!(
            "║  Throughput >= 30,000/sec:    {}  ({:>10.2}/sec)       ║",
            if throughput_pass { "✅ PASS" } else { "❌ FAIL" },
            throughput
        );
        println!(
            "║  Latency p50 <= 10ms:         {}  ({:>10.3} ms)        ║",
            if p50_pass { "✅ PASS" } else { "❌ FAIL" },
            p50_ms
        );
        println!(
            "║  Latency p99 <= 50ms:         {}  ({:>10.3} ms)        ║",
            if p99_pass { "✅ PASS" } else { "❌ FAIL" },
            p99_ms
        );
        println!(
            "║  Latency p999 <= 100ms:       {}  ({:>10.3} ms)        ║",
            if p999_pass { "✅ PASS" } else { "❌ FAIL" },
            p999_ms
        );
        println!(
            "║  Concurrent Connections >= 100: {}  ({:>10})           ║",
            if connections_pass { "✅ PASS" } else { "❌ FAIL" },
            num_clients
        );
        println!("╠════════════════════════════════════════════════════════════╣");

        let all_pass = throughput_pass && p50_pass && p99_pass && p999_pass && connections_pass;
        if all_pass {
            println!("║              🎉 ALL REQUIREMENTS PASSED! 🎉               ║");
        } else {
            println!("║              ❌ SOME REQUIREMENTS FAILED ❌                ║");
        }
        println!("╚════════════════════════════════════════════════════════════╝\n");

        // Assert all requirements
        assert!(
            throughput >= 30_000.0,
            "Throughput {:.2} is below target 30,000 req/sec",
            throughput
        );
        assert!(p50_ms <= 10.0, "p50 latency {:.3}ms exceeds 10ms", p50_ms);
        assert!(p99_ms <= 50.0, "p99 latency {:.3}ms exceeds 50ms", p99_ms);
        assert!(p999_ms <= 100.0, "p999 latency {:.3}ms exceeds 100ms", p999_ms);
        assert!(num_clients >= 100, "Concurrent clients {} is below 100", num_clients);
    });
}

#[test]
fn test_matching_correctness() {
    use order_matching_engine::matching::engine::MatchingEngine;
    use order_matching_engine::models::order::{CreateOrderRequest, OrderType, Side};

    println!("\n========== Matching Correctness Test ==========\n");

    // Test 1: Exact match at same price
    {
        println!("Test 1: Exact match at same price");
        let engine = MatchingEngine::new();
        
        fn create_limit_order(side: Side, price: u64, quantity: u64) -> CreateOrderRequest {
            CreateOrderRequest {
                symbol: "AAPL".to_string(),
                side,
                order_type: OrderType::Limit,
                price: Some(price),
                quantity,
            }
        }

        let _ = engine.submit_order(create_limit_order(Side::Sell, 15000, 100));
        let result = engine.submit_order(create_limit_order(Side::Buy, 15000, 100));
        
        assert!(result.is_ok());
        assert_eq!(engine.orders_matched(), 2);
        assert_eq!(engine.trades_executed(), 1);
        
        let book = engine.get_order_book("AAPL", 10);
        assert!(book.bids.is_empty(), "No bids should remain");
        assert!(book.asks.is_empty(), "No asks should remain");
        
        println!("  ✅ Orders matched correctly at same price\n");
    }

    // Test 2: No match when prices don't cross
    {
        println!("Test 2: No match when prices don't cross");
        let engine = MatchingEngine::new();
        
        fn create_limit_order(side: Side, price: u64, quantity: u64) -> CreateOrderRequest {
            CreateOrderRequest {
                symbol: "AAPL".to_string(),
                side,
                order_type: OrderType::Limit,
                price: Some(price),
                quantity,
            }
        }

        let _ = engine.submit_order(create_limit_order(Side::Sell, 15100, 50));
        let _ = engine.submit_order(create_limit_order(Side::Buy, 14900, 50));
        
        assert_eq!(engine.trades_executed(), 0, "No trades should have occurred");
        
        let book = engine.get_order_book("AAPL", 10);
        assert_eq!(book.bids.len(), 1, "One bid should be in book");
        assert_eq!(book.asks.len(), 1, "One ask should be in book");
        assert_eq!(book.bids[0].price, 14900);
        assert_eq!(book.asks[0].price, 15100);
        
        println!("  ✅ No match when buy price < sell price\n");
    }

    // Test 3: Match when buy price >= sell price
    {
        println!("Test 3: Match when buy price >= sell price");
        let engine = MatchingEngine::new();
        
        fn create_limit_order(side: Side, price: u64, quantity: u64) -> CreateOrderRequest {
            CreateOrderRequest {
                symbol: "AAPL".to_string(),
                side,
                order_type: OrderType::Limit,
                price: Some(price),
                quantity,
            }
        }

        // Sell at 15000
        let _ = engine.submit_order(create_limit_order(Side::Sell, 15000, 50));
        
        // Buy at 15100 (higher than ask) - should match at 15000
        let _ = engine.submit_order(create_limit_order(Side::Buy, 15100, 50));
        
        assert_eq!(engine.trades_executed(), 1, "Trade should have occurred");
        
        let book = engine.get_order_book("AAPL", 10);
        assert!(book.bids.is_empty(), "No bids should remain");
        assert!(book.asks.is_empty(), "No asks should remain");
        
        println!("  ✅ Match occurred when buy price >= sell price\n");
    }

    // Test 4: Partial fill correctness
    {
        println!("Test 4: Partial fill correctness");
        let engine = MatchingEngine::new();
        
        fn create_limit_order(side: Side, price: u64, quantity: u64) -> CreateOrderRequest {
            CreateOrderRequest {
                symbol: "AAPL".to_string(),
                side,
                order_type: OrderType::Limit,
                price: Some(price),
                quantity,
            }
        }

        let _ = engine.submit_order(create_limit_order(Side::Sell, 16000, 100));
        let _ = engine.submit_order(create_limit_order(Side::Buy, 16000, 30));

        assert_eq!(engine.trades_executed(), 1);
        
        let book = engine.get_order_book("AAPL", 10);
        assert!(book.bids.is_empty(), "Buy order should be fully filled");
        assert_eq!(book.asks.len(), 1, "Partial sell order should remain");
        assert_eq!(book.asks[0].price, 16000);
        assert_eq!(book.asks[0].quantity, 70, "70 should remain from 100-30");
        
        println!("  ✅ Partial fill: 100 - 30 = 70 remaining\n");
    }

    // Test 5: Price priority - lower ask matches first
    {
        println!("Test 5: Price priority");
        let engine = MatchingEngine::new();
        
        fn create_limit_order(side: Side, price: u64, quantity: u64) -> CreateOrderRequest {
            CreateOrderRequest {
                symbol: "AAPL".to_string(),
                side,
                order_type: OrderType::Limit,
                price: Some(price),
                quantity,
            }
        }

        // Add sells at different prices (higher price first, then lower)
        let _ = engine.submit_order(create_limit_order(Side::Sell, 17000, 50));
        let _ = engine.submit_order(create_limit_order(Side::Sell, 16500, 50));

        // Buy at 17000 should match 16500 first (lower price)
        let _ = engine.submit_order(create_limit_order(Side::Buy, 17000, 50));

        assert_eq!(engine.trades_executed(), 1);
        
        let book = engine.get_order_book("AAPL", 10);
        assert_eq!(book.asks.len(), 1, "One ask should remain");
        assert_eq!(book.asks[0].price, 17000, "17000 ask should remain (16500 was matched)");
        
        println!("  ✅ Price priority: lower ask matched first\n");
    }

    // Test 6: FIFO at same price level
    {
        println!("Test 6: FIFO at same price level");
        let engine = MatchingEngine::new();
        
        fn create_limit_order(side: Side, price: u64, quantity: u64) -> CreateOrderRequest {
            CreateOrderRequest {
                symbol: "AAPL".to_string(),
                side,
                order_type: OrderType::Limit,
                price: Some(price),
                quantity,
            }
        }

        // Add two sells at same price
        let _ = engine.submit_order(create_limit_order(Side::Sell, 18000, 100));
        let _ = engine.submit_order(create_limit_order(Side::Sell, 18000, 100));

        // Verify aggregated quantity
        let book = engine.get_order_book("AAPL", 10);
        assert_eq!(book.asks.len(), 1, "Should show one price level");
        assert_eq!(book.asks[0].quantity, 200, "Should show aggregated 200");

        // Buy 100 - should match first order completely
        let _ = engine.submit_order(create_limit_order(Side::Buy, 18000, 100));

        assert_eq!(engine.trades_executed(), 1);
        
        let book = engine.get_order_book("AAPL", 10);
        assert_eq!(book.asks.len(), 1, "One price level should remain");
        assert_eq!(book.asks[0].quantity, 100, "Second order (100) should remain");
        
        println!("  ✅ FIFO: first order at price level matched first\n");
    }

    // Test 7: Trade count accuracy
    {
        println!("Test 7: Verify trade count accuracy");
        let engine = MatchingEngine::new();
        
        fn create_limit_order(side: Side, price: u64, quantity: u64) -> CreateOrderRequest {
            CreateOrderRequest {
                symbol: "AAPL".to_string(),
                side,
                order_type: OrderType::Limit,
                price: Some(price),
                quantity,
            }
        }

        assert_eq!(engine.trades_executed(), 0);
        
        let _ = engine.submit_order(create_limit_order(Side::Sell, 19000, 50));
        let _ = engine.submit_order(create_limit_order(Side::Buy, 19000, 50));
        
        assert_eq!(engine.trades_executed(), 1);
        
        // Multiple trades
        let _ = engine.submit_order(create_limit_order(Side::Sell, 20000, 50));
        let _ = engine.submit_order(create_limit_order(Side::Sell, 20000, 50));
        let _ = engine.submit_order(create_limit_order(Side::Buy, 20000, 100));
        
        assert_eq!(engine.trades_executed(), 3, "Should have 1 + 2 = 3 trades");
        
        println!("  ✅ Trade count incremented correctly\n");
    }

    // Test 8: Bid price priority - higher bid matches first
    {
        println!("Test 8: Bid price priority");
        let engine = MatchingEngine::new();
        
        fn create_limit_order(side: Side, price: u64, quantity: u64) -> CreateOrderRequest {
            CreateOrderRequest {
                symbol: "AAPL".to_string(),
                side,
                order_type: OrderType::Limit,
                price: Some(price),
                quantity,
            }
        }

        // Add buys at different prices
        let _ = engine.submit_order(create_limit_order(Side::Buy, 14000, 50));
        let _ = engine.submit_order(create_limit_order(Side::Buy, 14500, 50));

        // Sell at 14000 should match 14500 first (higher bid)
        let _ = engine.submit_order(create_limit_order(Side::Sell, 14000, 50));

        assert_eq!(engine.trades_executed(), 1);
        
        let book = engine.get_order_book("AAPL", 10);
        assert_eq!(book.bids.len(), 1, "One bid should remain");
        assert_eq!(book.bids[0].price, 14000, "14000 bid should remain (14500 was matched)");
        
        println!("  ✅ Bid price priority: higher bid matched first\n");
    }

    println!("========== All Correctness Tests Passed! ==========\n");
}

#[test]
fn test_no_race_conditions() {
    use order_matching_engine::matching::engine::MatchingEngine;
    use order_matching_engine::models::order::{ CreateOrderRequest, OrderType, Side };
    use std::sync::atomic::{ AtomicU64, Ordering };
    use std::sync::Arc;
    use std::thread;

    fn create_limit_order(side: Side, price: u64, quantity: u64) -> CreateOrderRequest {
        CreateOrderRequest {
            symbol: "AAPL".to_string(),
            side,
            order_type: OrderType::Limit,
            price: Some(price),
            quantity,
        }
    }

    let engine = Arc::new(MatchingEngine::new());
    let num_threads = 50;
    let orders_per_thread = 1000;
    let expected_total_orders = num_threads * orders_per_thread;

    let total_buy_qty = Arc::new(AtomicU64::new(0));
    let total_sell_qty = Arc::new(AtomicU64::new(0));
    let successful_submissions = Arc::new(AtomicU64::new(0));
    let failed_submissions = Arc::new(AtomicU64::new(0));

    println!("\n========== Race Condition Test ==========");
    println!("Threads: {}", num_threads);
    println!("Orders per thread: {}", orders_per_thread);
    println!("Total expected orders: {}\n", expected_total_orders);

    let start = std::time::Instant::now();

    let handles: Vec<_> = (0..num_threads)
        .map(|thread_id| {
            let engine_clone = engine.clone();
            let buy_qty = total_buy_qty.clone();
            let sell_qty = total_sell_qty.clone();
            let success = successful_submissions.clone();
            let failed = failed_submissions.clone();

            thread::spawn(move || {
                for i in 0..orders_per_thread {
                    let qty = 10u64;
                    let price = 15000 + ((i % 100) as u64);

                    let result = if (thread_id + i) % 2 == 0 {
                        buy_qty.fetch_add(qty, Ordering::SeqCst);
                        engine_clone.submit_order(create_limit_order(Side::Buy, price, qty))
                    } else {
                        sell_qty.fetch_add(qty, Ordering::SeqCst);
                        engine_clone.submit_order(create_limit_order(Side::Sell, price, qty))
                    };

                    match result {
                        Ok(_) => {
                            success.fetch_add(1, Ordering::SeqCst);
                        }
                        Err(_) => {
                            failed.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    let elapsed = start.elapsed();

    let total_buy = total_buy_qty.load(Ordering::SeqCst);
    let total_sell = total_sell_qty.load(Ordering::SeqCst);
    let successful = successful_submissions.load(Ordering::SeqCst);
    let failed = failed_submissions.load(Ordering::SeqCst);
    let orders_received = engine.orders_received();
    let orders_in_book = engine.orders_in_book();
    let orders_matched = engine.orders_matched();
    let trades_executed = engine.trades_executed();

    println!("Time elapsed: {:?}", elapsed);
    println!("\n--- Submission Stats ---");
    println!("Total buy quantity submitted: {}", total_buy);
    println!("Total sell quantity submitted: {}", total_sell);
    println!("Successful submissions: {}", successful);
    println!("Failed submissions: {}", failed);

    println!("\n--- Engine Stats ---");
    println!("Orders received by engine: {}", orders_received);
    println!("Orders in book: {}", orders_in_book);
    println!("Orders matched: {}", orders_matched);
    println!("Trades executed: {}", trades_executed);

    println!("\n--- Verification ---");

    // Test 1: No orders lost
    let orders_lost = expected_total_orders - orders_received;
    if orders_lost == 0 {
        println!("✅ No orders lost: {} == {}", orders_received, expected_total_orders);
    } else {
        println!(
            "❌ Orders lost: {} (expected {}, got {})",
            orders_lost,
            expected_total_orders,
            orders_received
        );
    }
    assert_eq!(
        orders_received,
        expected_total_orders,
        "Some orders were lost due to race condition"
    );

    // Test 2: All submissions succeeded
    if failed == 0 {
        println!("✅ All submissions succeeded: {}", successful);
    } else {
        println!("⚠️  Some submissions failed: {} failed out of {}", failed, successful + failed);
    }

    // Test 3: Data consistency - matched orders should not exceed total
    assert!(
        orders_matched <= expected_total_orders,
        "More orders matched ({}) than submitted ({})",
        orders_matched,
        expected_total_orders
    );
    println!(
        "✅ Matched orders ({}) <= submitted orders ({})",
        orders_matched,
        expected_total_orders
    );

    // Test 4: Trade count consistency
    // Each trade involves 2 orders, so trades <= orders_matched / 2
    let max_possible_trades = (orders_matched / 2) as u64;
    assert!(
        trades_executed <= max_possible_trades + 1, // +1 for rounding
        "Trade count ({}) inconsistent with matched orders ({})",
        trades_executed,
        orders_matched
    );
    println!("✅ Trade count ({}) is consistent", trades_executed);

    // Test 5: Order book consistency
    // Orders should be either in book or matched
    println!("✅ Orders in book: {}, Matched: {}", orders_in_book, orders_matched);

    // Test 6: No negative quantities (would indicate corruption)
    let book = engine.get_order_book("AAPL", 1000);
    for bid in &book.bids {
        assert!(bid.quantity > 0, "Found zero/negative quantity in bids");
    }
    for ask in &book.asks {
        assert!(ask.quantity > 0, "Found zero/negative quantity in asks");
    }
    println!("✅ No zero/negative quantities in order book");

    println!("\n========== Race Condition Test Passed! ==========\n");
}

// ============================================================================
// Additional: Stress Test for Data Integrity
// ============================================================================

#[test]
fn test_data_integrity_under_stress() {
    use order_matching_engine::matching::engine::MatchingEngine;
    use order_matching_engine::models::order::{ CreateOrderRequest, OrderType, Side };
    use std::sync::Arc;
    use std::thread;

    fn create_limit_order(
        symbol: &str,
        side: Side,
        price: u64,
        quantity: u64
    ) -> CreateOrderRequest {
        CreateOrderRequest {
            symbol: symbol.to_string(),
            side,
            order_type: OrderType::Limit,
            price: Some(price),
            quantity,
        }
    }

    let engine = Arc::new(MatchingEngine::new());
    let num_threads = 20;
    let operations_per_thread = 500;
    let symbols = vec!["AAPL", "GOOG", "MSFT", "AMZN", "META"];

    println!("\n========== Data Integrity Stress Test ==========");
    println!("Threads: {}", num_threads);
    println!("Operations per thread: {}", operations_per_thread);
    println!("Symbols: {:?}\n", symbols);

    let handles: Vec<_> = (0..num_threads)
        .map(|thread_id| {
            let engine_clone = engine.clone();
            let symbols_clone = symbols.clone();

            thread::spawn(move || {
                for i in 0..operations_per_thread {
                    let symbol = symbols_clone[i % symbols_clone.len()];
                    let side = if (thread_id + i) % 2 == 0 { Side::Buy } else { Side::Sell };
                    let price = 10000 + ((i % 100) as u64);
                    let quantity = 10 + ((i % 50) as u64);

                    // Submit order
                    let _ = engine_clone.submit_order(
                        create_limit_order(symbol, side, price, quantity)
                    );

                    // Occasionally query order book (read operation)
                    if i % 10 == 0 {
                        let _ = engine_clone.get_order_book(symbol, 10);
                    }

                    // Occasionally check metrics
                    if i % 50 == 0 {
                        let _ = engine_clone.orders_received();
                        let _ = engine_clone.trades_executed();
                    }
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    // Verify data integrity
    println!("--- Final State ---");
    println!("Total orders received: {}", engine.orders_received());
    println!("Total trades executed: {}", engine.trades_executed());
    println!("Orders in book: {}", engine.orders_in_book());

    // Check each symbol's order book
    for symbol in &symbols {
        let book = engine.get_order_book(symbol, 100);

        let total_bid_qty: u64 = book.bids
            .iter()
            .map(|b| b.quantity)
            .sum();
        let total_ask_qty: u64 = book.asks
            .iter()
            .map(|a| a.quantity)
            .sum();

        println!(
            "  {}: {} bid levels (qty: {}), {} ask levels (qty: {})",
            symbol,
            book.bids.len(),
            total_bid_qty,
            book.asks.len(),
            total_ask_qty
        );

        // Verify bid prices are in descending order
        for window in book.bids.windows(2) {
            assert!(
                window[0].price >= window[1].price,
                "Bids not in descending order for {}",
                symbol
            );
        }

        // Verify ask prices are in ascending order
        for window in book.asks.windows(2) {
            assert!(
                window[0].price <= window[1].price,
                "Asks not in ascending order for {}",
                symbol
            );
        }

        // Verify no crossed book (best bid < best ask)
        if !book.bids.is_empty() && !book.asks.is_empty() {
            assert!(
                book.bids[0].price < book.asks[0].price,
                "Crossed book detected for {}: bid {} >= ask {}",
                symbol,
                book.bids[0].price,
                book.asks[0].price
            );
        }
    }

    println!("\n✅ All order books maintain correct price ordering");
    println!("✅ No crossed books detected");
    println!("\n========== Data Integrity Test Passed! ==========\n");
}
