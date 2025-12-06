# Order Matching Engine

Prerequisites
- Install Rust and Cargo: https://rustup.rs

Quick start
1. Build:
   ```
   cargo build
   ```
2. Run the server (starts on localhost:8080):
   ```
   cargo run
   ```

API Endpoints
1. Submit Order
   - POST /api/v1/orders
   - Description: Submit a new order (JSON body).
   - Example:
     ```
     curl -X POST http://localhost:8080/api/v1/orders \
       -H "Content-Type: application/json" \
       -d '{"symbol":"AAPL","side":"BUY","type":"LIMIT","price":50000,"quantity":100}'
     ```

2. Cancel Order
   - DELETE /api/v1/orders/{order_id}
   - Description: Cancel an existing order by order_id.
   - Example:
     ```
     curl -X DELETE http://localhost:8080/api/v1/orders/ORDER_ID_HERE
     ```

3. Get Order Book
   - GET /api/v1/orderbook/{symbol}?depth=10
   - Description: Retrieve the order book for a symbol. Optional `depth` query param controls levels returned (example uses 10).
   - Example:
     ```
     curl "http://localhost:8080/api/v1/orderbook/BTCUSD?depth=10"
     ```

4. Get Order Status
   - GET /api/v1/orders/{order_id}
   - Description: Retrieve status/details of an order by order_id.
   - Example:
     ```
     curl http://localhost:8080/api/v1/orders/ORDER_ID_HERE
     ```

5. Health Check
   - GET /health
   - Description: Basic liveness/health endpoint.
   - Example:
     ```
     curl http://localhost:8080/health
     ```

6. Metrics Endpoint
   - GET /metrics
   - Description: Exposes metrics (Prometheus-compatible).
   - Example:
     ```
     curl http://localhost:8080/metrics
     ```

Notes
- The server listens on localhost:8080 by default.
- Request/response JSON schemas depend on the implementation; consult code for exact fields accepted/returned.


# Key Design Decisions:

1. Data Structures for Optimal Performance
Order Book Structure:

BTreeMap for Price Levels: Chosen over HashMap because it maintains sorted order, enabling O(1) access to best bid (highest) and best ask (lowest) prices. This is critical for price-priority matching.
VecDeque for Orders at Each Price Level: Provides O(1) push/pop operations for FIFO (time-priority) order matching within a price level.
HashMap for Order Lookups: Enables O(1) order cancellation and status queries by order ID.

```text
Complexity Analysis:
├── Insert Order:      O(log n) - BTreeMap insertion
├── Cancel Order:      O(1) lookup + O(k) removal from VecDeque
├── Get Best Bid/Ask:  O(1) - BTreeMap first/last
├── Match Orders:      O(m) - where m is number of matched orders
└── Order Status:      O(1) - HashMap lookup
```

2. Thread Safety with RwLock
Multiple concurrent readers: GET requests (order status, order book, metrics) don't block each other
Exclusive writer access: Only one write operation (submit/cancel) at a time
No poisoning: parking_lot doesn't poison locks on panic, improving reliability
Trade-off: Under extremely high write contention, this can become a bottleneck. For production systems handling millions of orders per second, consider sharding order books by symbol.

3. Market Order Rejection Strategy
Market orders are rejected if insufficient liquidity exists, rather than partially filling:

Rationale: Partial fills on market orders can lead to unexpected execution prices and quantities for traders
Implementation: Check total available quantity before attempting to match
Alternative considered: Partial fills with "Immediate-or-Cancel" semantics, but rejected for simplicity
4. In-Memory Storage with Dual Storage Strategy
Two separate storage areas serve different purposes:

Order Book: Contains only active orders (Accepted, PartialFill status). Orders are removed when fully filled or cancelled.
Order Storage: Contains all orders ever submitted, enabling status queries on filled/cancelled orders.
This separation optimizes matching performance while maintaining order history for queries.

5. Price Representation
Prices are stored as u64 integers representing the smallest unit (e.g., cents):

Why not floats?: Floating-point arithmetic can introduce rounding errors in financial calculations
Example: $150.50 is stored as 15050
Benefit: Exact comparisons and arithmetic without precision loss

6. Metrics Collection
We use HDR Histogram for latency tracking:

7. Async HTTP with Axum
Axum was chosen as the web framework for:

Performance: Built on Tokio and Hyper, known for high throughput
Ergonomics: Clean API with strong typing via extractors
Ecosystem: Seamless integration with Tower middleware
8. Order Matching Algorithm
The matching algorithm follows price-time priority:

```text
For a BUY order:
1. Find all ASK prices <= BUY price (starting from lowest)
2. At each price level, match orders in FIFO order
3. Continue until BUY order is filled or no more matching ASKs

For a SELL order:
1. Find all BID prices >= SELL price (starting from highest)
2. At each price level, match orders in FIFO order
3. Continue until SELL order is filled or no more matching BIDs
```

# Testing
Run unit, integration and performance tests with:
```
cargo test
```
  - Three performance tests will fail when cargo test is run normally. These tests need to be run individually using `cargo test --release test_http_latency_measurements -- --nocapture`, `cargo test --release test_http_throughput_concurrent -- --nocapture` and `cargo test --release load_test_60_seconds_http -- --nocapture` respectively.

