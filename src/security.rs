//! Credential transport boundaries shared by the SDK and agent interfaces.
use crate::{Result, YappinError};
use reqwest::header::HeaderValue;
use serde_json::Value;
use url::{Host, Url};

/// Local HTTP is allowed only for explicitly configured loopback development servers.
pub(crate) fn endpoint(raw: &str, websocket: bool) -> Result<Url> {
    let url = Url::parse(raw).map_err(|_| invalid_endpoint())?;
    let loopback = match url.host() {
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        Some(Host::Domain("localhost")) => true,
        _ => false,
    };
    let secure = if websocket { "wss" } else { "https" };
    let local = if websocket { "ws" } else { "http" };
    if (url.scheme() != secure && !(url.scheme() == local && loopback))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.host().is_none()
    {
        return Err(invalid_endpoint());
    }
    Ok(url)
}
fn invalid_endpoint() -> YappinError {
    YappinError::Config("credential endpoint must use TLS (loopback development excepted), without userinfo, query, or fragment".into())
}
/// Resolve only paths inside the configured API prefix; never reinterpret input as an authority.
pub(crate) fn api_url(base: &str, path: &str) -> Result<Url> {
    let base = endpoint(base, false)?;
    if !path.starts_with('/')
        || path.starts_with("//")
        || path.contains('\\')
        || path.contains('#')
        || path.chars().any(char::is_control)
    {
        return Err(YappinError::validation(
            "path",
            "requires a relative API path; URL authorities and backslashes are forbidden",
        ));
    }
    let prefix = base.path().trim_end_matches('/');
    let target = Url::parse(&format!("{}{path}", base.as_str().trim_end_matches('/')))
        .map_err(|_| YappinError::validation("path", "invalid API path"))?;
    if target.origin() != base.origin()
        || (!prefix.is_empty() && !target.path().starts_with(&format!("{prefix}/")))
    {
        return Err(YappinError::validation(
            "path",
            "path escapes the configured API prefix",
        ));
    }
    Ok(target)
}
pub(crate) fn authorization(scheme: &str, secret: &str) -> Result<HeaderValue> {
    if secret.is_empty() || !secret.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(YappinError::Config(
            "credential is empty or contains invalid characters".into(),
        ));
    }
    let mut value = HeaderValue::from_str(&format!("{scheme} {secret}"))
        .map_err(|_| YappinError::Config("invalid credential header".into()))?;
    value.set_sensitive(true);
    Ok(value)
}
pub(crate) fn scrub(text: &str, secrets: &[&str]) -> String {
    let mut text = text.to_owned();
    for secret in secrets.iter().filter(|s| !s.is_empty()) {
        text = text.replace(secret, "[REDACTED]");
    }
    text
}
/// Redact exact configured credentials in values AND keys, including nested objects.
/// This is a logging aid, not a sandbox against trusted code that already has the credential.
pub fn redact_json(value: Value, secrets: &[&str]) -> Value {
    match value {
        Value::String(s) => Value::String(scrub(&s, secrets)),
        Value::Array(a) => Value::Array(a.into_iter().map(|v| redact_json(v, secrets)).collect()),
        Value::Object(o) => Value::Object(
            o.into_iter()
                .map(|(k, v)| (scrub(&k, secrets), redact_json(v, secrets)))
                .collect(),
        ),
        other => other,
    }
}

/// Redact credential-bearing fields in diagnostic representations of raw events.
pub(crate) fn redact_fields(value: Value) -> Value {
    match value {
        Value::Object(o) => Value::Object(
            o.into_iter()
                .map(|(k, v)| {
                    let secret = matches!(
                        k.to_ascii_lowercase().as_str(),
                        "token"
                            | "access_token"
                            | "refresh_token"
                            | "authorization"
                            | "client_secret"
                    );
                    (
                        k,
                        if secret {
                            Value::String("[REDACTED]".into())
                        } else {
                            redact_fields(v)
                        },
                    )
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.into_iter().map(redact_fields).collect()),
        other => other,
    }
}

pub(crate) fn json_error(error: serde_json::Error, secrets: &[&str]) -> YappinError {
    YappinError::Json(<serde_json::Error as serde::de::Error>::custom(scrub(
        &error.to_string(),
        secrets,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_cannot_reinterpret_the_origin_or_escape_the_api_prefix() {
        for path in [
            "@attacker.example/x",
            "//attacker.example/x",
            "/\\attacker.example/x",
            "/../private",
            "/%2e%2e/private",
            "/foo#fragment",
            "/foo\r\nheader",
        ] {
            assert!(
                api_url("https://www.yapin.gg/api/bot/v1", path).is_err(),
                "{path}"
            );
        }
        let url = api_url("https://www.yapin.gg/api/bot/v1", "/channels?limit=10").unwrap();
        assert_eq!(url.host_str(), Some("www.yapin.gg"));
        assert_eq!(url.path(), "/api/bot/v1/channels");
        assert!(api_url("https://www.yapin.gg", "@attacker.example").is_err());
    }
    #[test]
    fn remote_credentials_require_encryption() {
        for url in [
            "http://attacker.example/api",
            "https://user:secret@www.yapin.gg/api",
            "https://www.yapin.gg/api?token=secret",
            "https://www.yapin.gg/api#fragment",
        ] {
            assert!(endpoint(url, false).is_err());
        }
        assert!(endpoint("ws://attacker.example/bot", true).is_err());
        assert!(endpoint("http://127.0.0.1:8080/api", false).is_ok());
        assert!(endpoint("ws://[::1]:8080/bot", true).is_ok());
    }
    #[test]
    fn credential_headers_are_sensitive_and_injection_errors_do_not_echo_tokens() {
        let header = authorization("Bot", "yb_private_fixture").unwrap();
        assert!(header.is_sensitive());
        assert!(!format!("{header:?}").contains("yb_private_fixture"));
        let err = authorization("Bot", "yb_private_fixture\r\nX: injected").unwrap_err();
        assert!(!format!("{err:?}").contains("yb_private_fixture"));
    }
    #[test]
    fn redaction_includes_object_keys_and_nesting() {
        let value = serde_json::json!({"yb_private_fixture":{"nested":["yb_private_fixture"]}});
        let redacted = redact_json(value, &["yb_private_fixture"]);
        assert!(!redacted.to_string().contains("yb_private_fixture"));
    }
}
