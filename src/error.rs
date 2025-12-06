use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

#[derive(Debug)]
pub enum AppError {
    /// Order not found (404)
    OrderNotFound(Uuid),
    /// Symbol not found (404)
    SymbolNotFound(String),
    /// Invalid order request (400)
    InvalidOrder(String),
    /// Cannot cancel filled order (400)
    CannotCancelFilledOrder(Uuid),
    /// Insufficient liquidity for market order (400)
    InsufficientLiquidity { available: u64, requested: u64 },
}

use uuid::Uuid;

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::OrderNotFound(id) => (
                StatusCode::NOT_FOUND,
                format!("Order not found: {}", id),
            ),
            AppError::SymbolNotFound(symbol) => (
                StatusCode::NOT_FOUND,
                format!("Symbol not found: {}", symbol),
            ),
            AppError::InvalidOrder(msg) => (
                StatusCode::BAD_REQUEST,
                format!("Invalid order: {}", msg),
            ),
            AppError::CannotCancelFilledOrder(id) => (
                StatusCode::BAD_REQUEST,
                format!("Cannot cancel: order {} already filled", id),
            ),
            AppError::InsufficientLiquidity { available, requested } => (
                StatusCode::BAD_REQUEST,
                format!(
                    "Insufficient liquidity: only {} available, requested {}",
                    available, requested
                ),
            ),
        };

        let body = Json(ErrorResponse { error: message });
        (status, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn test_order_not_found_status() {
        let error = AppError::OrderNotFound(Uuid::new_v4());
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_invalid_order_status() {
        let error = AppError::InvalidOrder("test error".to_string());
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_insufficient_liquidity_status() {
        let error = AppError::InsufficientLiquidity {
            available: 50,
            requested: 100,
        };
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_cannot_cancel_filled_status() {
        let error = AppError::CannotCancelFilledOrder(Uuid::new_v4());
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}