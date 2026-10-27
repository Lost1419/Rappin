//! Shared retry deadlines. Requests are never serialized unless a server-advised wait applies.
use reqwest::Method;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, time::Duration};
use tokio::{sync::Mutex, time::Instant};

#[derive(Default)]
pub(crate) struct RateLimits {
    state: Mutex<State>,
}
#[derive(Default)]
struct State {
    routes: HashMap<[u8; 32], Instant>,
    global: Option<Instant>,
}
// Only literal API templates enter diagnostics. Even IDs equal to route keywords stay hidden.
const ROUTES: &[&str] = &[
    "/me",
    "/me/avatar",
    "/me/banner",
    "/me/status",
    "/events",
    "/commands",
    "/servers",
    "/servers/:id",
    "/servers/:id/channels",
    "/channels",
    "/servers/:id/categories",
    "/servers/:id/categories/:id",
    "/servers/:id/roles",
    "/servers/:id/members",
    "/servers/:id/members/:id",
    "/servers/:id/members/:id/roles/:id",
    "/servers/:id/kick",
    "/servers/:id/bans",
    "/servers/:id/bans/:id",
    "/servers/:id/timeouts",
    "/servers/:id/voice",
    "/servers/:id/voice/move",
    "/servers/:id/voice/disconnect",
    "/servers/:id/voice/join",
    "/servers/:id/voice/leave",
    "/channels/:id",
    "/channels/:id/messages",
    "/channels/:id/messages/bulk-delete",
    "/channels/:id/messages/:id",
    "/channels/:id/messages/:id/pin",
    "/channels/:id/messages/:id/reactions",
    "/channels/:id/messages/:id/reactions/:id",
    "/interactions/:id/:id/callback",
    "/users/:id",
    "/users/:id/dm",
];
pub(crate) fn route_template(path: &str) -> String {
    let path = path.split('?').next().unwrap_or(path);
    let parts: Vec<_> = path.trim_matches('/').split('/').collect();
    ROUTES
        .iter()
        .find(|template| {
            let shape: Vec<_> = template.trim_matches('/').split('/').collect();
            shape.len() == parts.len()
                && shape
                    .iter()
                    .zip(&parts)
                    .all(|(shape, part)| *shape == ":id" || shape == part)
        })
        .unwrap_or(&"/:unknown")
        .to_string()
}
pub(crate) fn key(method: &Method, path: &str) -> [u8; 32] {
    let clean = path.split('?').next().unwrap_or(path);
    let parts: Vec<_> = clean.trim_matches('/').split('/').collect();
    let mut hash = Sha256::new();
    hash.update(method.as_str().as_bytes());
    hash.update(route_template(clean).as_bytes());
    // Major server/channel scope. Interaction paths are hashed in full, never retained in diagnostics.
    if parts.first() == Some(&"interactions") {
        hash.update(clean.as_bytes());
    } else if let Some(major) = parts.get(1) {
        hash.update(major.as_bytes());
    }
    hash.finalize().into()
}
pub(crate) fn message_send(method: &Method, path: &str) -> bool {
    let parts: Vec<_> = path
        .split('?')
        .next()
        .unwrap_or(path)
        .trim_matches('/')
        .split('/')
        .collect();
    method == Method::POST && parts.len() == 3 && parts[0] == "channels" && parts[2] == "messages"
}
pub(crate) fn retry_after(value: &str) -> Option<Duration> {
    if let Ok(seconds) = value.parse::<f64>() {
        return Duration::try_from_secs_f64(seconds).ok();
    }
    let date = chrono::DateTime::parse_from_rfc2822(value)
        .ok()?
        .with_timezone(&chrono::Utc);
    Some(
        (date - chrono::Utc::now())
            .to_std()
            .unwrap_or(Duration::ZERO),
    )
}
impl RateLimits {
    pub(crate) async fn wait(&self, key: [u8; 32], message: bool) -> Duration {
        let started = Instant::now();
        loop {
            let deadline = {
                let mut state = self.state.lock().await;
                let now = Instant::now();
                state.routes.retain(|_, until| *until > now);
                let route = state.routes.get(&key).copied();
                let global = if message {
                    state.global.filter(|until| *until > now)
                } else {
                    None
                };
                route.into_iter().chain(global).max()
            };
            match deadline {
                Some(until) => tokio::time::sleep_until(until).await,
                None => return started.elapsed(),
            }
        }
    }
    pub(crate) async fn delay(&self, key: [u8; 32], duration: Duration, global: bool) {
        let now = Instant::now();
        let until = now + duration;
        let mut state = self.state.lock().await;
        state.routes.retain(|_, until| *until > now);
        if state.routes.len() < 1024 || state.routes.contains_key(&key) {
            let current = state.routes.entry(key).or_insert(until);
            *current = (*current).max(until);
        }
        if global {
            state.global = Some(state.global.map_or(until, |current| current.max(until)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn scoped_waits_and_global_message_waits_do_not_block_callbacks() {
        let limits = RateLimits::default();
        let first = key(&Method::POST, "/channels/one/messages");
        let second = key(&Method::POST, "/channels/two/messages");
        limits.delay(first, Duration::from_millis(40), false).await;
        tokio::time::timeout(Duration::from_millis(20), limits.wait(second, true))
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(10), limits.wait(first, true))
                .await
                .is_err()
        );
        limits.delay(first, Duration::from_millis(40), true).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(10), limits.wait(second, true))
                .await
                .is_err()
        );
        limits.wait(second, false).await; // Non-message endpoints bypass the global message gate.
        assert!(!message_send(
            &Method::POST,
            "/interactions/id/token/callback"
        ));
        limits.wait(first, true).await;
    }
    #[test]
    fn retry_headers_and_route_templates_do_not_expose_secrets() {
        assert_eq!(retry_after("0.25"), Some(Duration::from_millis(250)));
        for bad in ["NaN", "inf", "-1", "invalid"] {
            assert!(retry_after(bad).is_none());
        }
        let date = (chrono::Utc::now() + chrono::Duration::seconds(30))
            .format("%a, %d %b %Y %H:%M:%S GMT")
            .to_string();
        let duration = retry_after(&date).unwrap();
        assert!(duration.as_secs() >= 28 && duration.as_secs() <= 30);
        assert_eq!(
            route_template("/interactions/me/channels/callback"),
            "/interactions/:id/:id/callback"
        );
        assert_eq!(route_template("/secret/arbitrary/path"), "/:unknown");
        assert_eq!(
            route_template("/interactions/123/secret/callback?token=secret"),
            "/interactions/:id/:id/callback"
        );
    }
}
