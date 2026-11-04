//! Axum middleware for inbound webhook verification.
//!
//! This module provides ready-to-use Axum middleware that verifies
//! webhook signatures before passing events to your handler.
//!
//! # Example
//!
//! ```no_run
//! use axum::{Router, routing::post};
//! use rappin::{webhook_middleware, WebhookClient};
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() {
//!     let secret = std::env::var("WEBHOOK_SECRET").unwrap();
//!     let bot_id = std::env::var("BOT_ID").unwrap();
//!
//!     let app = Router::new()
//!         .route("/webhook", post(webhook_middleware::route(
//!             secret.clone(),
//!             bot_id.clone(),
//!             |event| Box::pin(async move {
//!                 // Handle verified event
//!                 println!("Received event: {:?}", event.kind);
//!                 Ok(())
//!             })
//!         )));
//!
//!     let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
//!     axum::serve(listener, app).await.unwrap();
//! }
//! ```

use crate::{Event, YappinError};
use axum::{
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::sync::Arc;

/// Configuration for webhook verification middleware.
#[derive(Clone)]
pub struct WebhookConfig {
    /// The webhook secret from the Yapin developer portal.
    pub secret: String,
    /// The expected bot ID (from X-Yapper-Bot header).
    pub bot_id: String,
    /// Maximum body size to accept (default 1 MiB).
    pub max_body_size: usize,
}

impl WebhookConfig {
    /// Create a new webhook configuration.
    pub fn new(secret: impl Into<String>, bot_id: impl Into<String>) -> Self {
        Self {
            secret: secret.into(),
            bot_id: bot_id.into(),
            max_body_size: 1024 * 1024, // 1 MiB
        }
    }

    /// Set the maximum body size.
    pub fn max_body_size(mut self, bytes: usize) -> Self {
        self.max_body_size = bytes;
        self
    }
}

/// Error response for webhook verification failures.
#[derive(Debug)]
pub struct WebhookError {
    pub status: StatusCode,
    pub message: String,
}

impl IntoResponse for WebhookError {
    fn into_response(self) -> Response {
        (self.status, self.message).into_response()
    }
}

/// Convert a YappinError to an axum Response.
fn yappin_error_to_response(err: YappinError) -> Response {
    let status = err
        .status_code()
        .and_then(|c| StatusCode::from_u16(c).ok())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, err.to_string()).into_response()
}

/// Axum middleware that verifies webhook signatures.
///
/// This middleware:
/// 1. Reads the raw request body
/// 2. Verifies the `X-Yapper-Signature` header against the secret
/// 3. Verifies the `X-Yapper-Bot` header matches the expected bot ID
/// 4. Parses the body as an `Event`
/// 5. Passes the verified event to the handler via request extensions
pub async fn verify_middleware(
    State(config): State<Arc<WebhookConfig>>,
    mut request: Request,
    next: Next,
) -> Result<Response, WebhookError> {
    let headers = request.headers().clone();

    // Extract and verify X-Yapper-Bot header
    let bot_id = headers
        .get("X-Yapper-Bot")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| WebhookError {
            status: StatusCode::BAD_REQUEST,
            message: "Missing X-Yapper-Bot header".into(),
        })?;

    if bot_id != config.bot_id {
        return Err(WebhookError {
            status: StatusCode::FORBIDDEN,
            message: "Invalid bot ID".into(),
        });
    }

    // Extract signature
    let signature = headers
        .get("X-Yapper-Signature")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| WebhookError {
            status: StatusCode::BAD_REQUEST,
            message: "Missing X-Yapper-Signature header".into(),
        })?;

    // Take the body out of the request
    let body = std::mem::take(request.body_mut());

    // Read body with size limit
    let body_bytes = axum::body::to_bytes(body, config.max_body_size)
        .await
        .map_err(|_| WebhookError {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            message: "Body too large".into(),
        })?;

    // Verify signature
    let event =
        crate::webhook::verify_event(&config.secret, signature, &body_bytes).map_err(|_| {
            WebhookError {
                status: StatusCode::FORBIDDEN,
                message: "Invalid webhook signature".into(),
            }
        })?;

    // Store verified event in request extensions for handler access
    request.extensions_mut().insert(event);

    // Restore body for downstream handlers
    *request.body_mut() = Body::from(body_bytes);

    Ok(next.run(request).await)
}

/// Extract a verified event from request extensions.
///
/// Use this in your handler to get the verified event:
/// ```ignore
/// async fn handler(req: Request) -> impl IntoResponse {
///     let event = webhook_middleware::get_verified_event(&req).unwrap();
///     // ...
/// }
/// ```
pub fn get_verified_event(request: &Request) -> Option<&Event> {
    request.extensions().get::<Event>()
}

/// Create a verified webhook handler.
///
/// This is a convenience function that creates a handler which automatically
/// extracts the verified event from request extensions.
pub fn verified_handler<F, R>(
    handler: F,
) -> impl Fn(Request) -> std::pin::Pin<Box<dyn std::future::Future<Output = Response> + Send>> + Clone
where
    F: Fn(
            Event,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<R, YappinError>> + Send>>
        + Clone
        + Send
        + Sync
        + 'static,
    R: IntoResponse,
{
    move |request: Request| {
        let event = get_verified_event(&request)
            .cloned()
            .expect("verified event should be present in extensions");
        let handler = handler.clone();
        Box::pin(async move {
            match handler(event).await {
                Ok(response) => response.into_response(),
                Err(err) => yappin_error_to_response(err),
            }
        })
    }
}

/// Convenience function to create a webhook route with verification.
///
/// # Example
/// ```no_run
/// use axum::Router;
/// use rappin::webhook_middleware;
///
/// let app = Router::new()
///     .route("/webhook", webhook_middleware::route(
///         std::env::var("WEBHOOK_SECRET").unwrap(),
///         std::env::var("BOT_ID").unwrap(),
///         |event| Box::pin(async move {
///             println!("Event: {:?}", event.kind);
///             Ok(())
///         })
///     ));
/// ```
pub fn route<F, R>(
    secret: impl Into<String>,
    bot_id: impl Into<String>,
    handler: F,
) -> axum::routing::MethodRouter
where
    F: Fn(
            Event,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<R, YappinError>> + Send>>
        + Clone
        + Send
        + Sync
        + 'static,
    R: IntoResponse,
{
    let config = Arc::new(WebhookConfig::new(secret, bot_id));
    axum::routing::post(move |req: Request| {
        let handler = handler.clone();
        async move {
            let event = match get_verified_event(&req).cloned() {
                Some(event) => event,
                None => {
                    return yappin_error_to_response(YappinError::Config(
                        "verified event missing".into(),
                    ))
                }
            };
            match handler(event).await {
                Ok(response) => response.into_response(),
                Err(err) => yappin_error_to_response(err),
            }
        }
    })
    .layer(axum::middleware::from_fn_with_state(
        config,
        verify_middleware,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        routing::post,
        Router,
    };
    use hmac::KeyInit;
    use tower::util::ServiceExt;

    #[tokio::test]
    async fn test_verify_middleware_valid() {
        let secret = "test_secret";
        let bot_id = "123456789";

        let event = serde_json::json!({
            "id": 1,
            "type": "message_create",
            "data": {}
        });
        let body = serde_json::to_vec(&event).unwrap();

        // Generate valid signature
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(&body);
        let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

        let config = Arc::new(WebhookConfig::new(secret, bot_id));
        let app = Router::new()
            .route(
                "/webhook",
                post(verified_handler(|_event| {
                    Box::pin(async { Ok::<(), YappinError>(()) })
                })),
            )
            .layer(axum::middleware::from_fn_with_state(
                config,
                verify_middleware,
            ));

        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("X-Yapper-Bot", bot_id)
            .header("X-Yapper-Signature", signature)
            .body(Body::from(body))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_verify_middleware_invalid_signature() {
        let secret = "test_secret";
        let bot_id = "123456789";

        let event = serde_json::json!({
            "id": 1,
            "type": "message_create",
            "data": {}
        });
        let body = serde_json::to_vec(&event).unwrap();

        let config = Arc::new(WebhookConfig::new(secret, bot_id));
        let app = Router::new()
            .route(
                "/webhook",
                post(verified_handler(|_event| {
                    Box::pin(async { Ok::<(), YappinError>(()) })
                })),
            )
            .layer(axum::middleware::from_fn_with_state(
                config,
                verify_middleware,
            ));

        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("X-Yapper-Bot", bot_id)
            .header("X-Yapper-Signature", "sha256=invalid")
            .body(Body::from(body))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_verify_middleware_wrong_bot_id() {
        let secret = "test_secret";
        let bot_id = "123456789";

        let event = serde_json::json!({
            "id": 1,
            "type": "message_create",
            "data": {}
        });
        let body = serde_json::to_vec(&event).unwrap();

        // Generate valid signature
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(&body);
        let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

        let config = Arc::new(WebhookConfig::new(secret, bot_id));
        let app = Router::new()
            .route(
                "/webhook",
                post(verified_handler(|_event| {
                    Box::pin(async { Ok::<(), YappinError>(()) })
                })),
            )
            .layer(axum::middleware::from_fn_with_state(
                config,
                verify_middleware,
            ));

        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("X-Yapper-Bot", "wrong_bot_id")
            .header("X-Yapper-Signature", signature)
            .body(Body::from(body))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
