//! OAuth2 authorization-code and S256 PKCE support for Login with Yapin'.
use crate::utils::DEFAULT_SITE_BASE;
use crate::{Result, YappinError};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::header;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Sensitive user authorization response; Debug is deliberately redacted and serialization still contains credentials.
///
/// # Example
///
/// ```rust
/// use rappin::oauth2::OAuth2Token;
/// fn expiry(token: &OAuth2Token) -> Option<std::time::Duration> {
///     token.expires_in.map(std::time::Duration::from_secs)
/// }
/// ```
pub struct OAuth2Token {
    /// Sensitive user bearer credential; never log it.
    pub access_token: String,
    #[serde(default)]
    /// Token type reported by the OAuth server.
    pub token_type: String,
    #[serde(default)]
    /// Access-token lifetime in seconds, when supplied.
    pub expires_in: Option<u64>,
    /// Reserved for API evolution; Yapin currently does not issue refresh tokens.
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    /// Granted scope string, when supplied.
    pub scope: Option<String>,
}
/// Identity returned directly by /api/oauth/me.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let user: rappin::oauth2::OAuth2User = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"username":"river","display_name":null,"avatar_url":null}))?;
/// assert_eq!(user.username, "river");
/// # Ok::<(), serde_json::Error>(())
/// ```
pub struct OAuth2User {
    /// Authorized user UUID.
    pub id: uuid::Uuid,
    /// Current user name.
    pub username: String,
    /// Optional display name.
    pub display_name: Option<String>,
    /// Optional user avatar URL.
    pub avatar_url: Option<String>,
}
/// Keep the verifier private in the login session and send only its challenge to authorize.
#[derive(Clone)]
/// # Example
///
/// ```rust
/// use rappin::oauth2::{OAuth2Client, Pkce};
/// let client = OAuth2Client::public("example-client-id", "http://localhost:3000/callback");
/// let pkce = Pkce::new();
/// let state = uuid::Uuid::new_v4().to_string();
/// let authorize = client.authorization_url_pkce(&state, &pkce);
/// assert!(authorize.contains("code_challenge_method=S256"));
/// assert!(!authorize.contains(pkce.verifier()));
/// // Keep state and verifier in the login session; verify returned state before exchanging code.
/// ```
pub struct Pkce {
    verifier: String,
}
impl Default for Pkce {
    fn default() -> Self {
        Self::new()
    }
}
impl Pkce {
    /// Generate a fresh private verifier for one login session.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::oauth2::{OAuth2Client, Pkce};
    /// let client = OAuth2Client::public("example-client-id", "http://localhost:3000/callback");
    /// let pkce = Pkce::new();
    /// let state = uuid::Uuid::new_v4().to_string();
    /// let authorize = client.authorization_url_pkce(&state, &pkce);
    /// assert!(authorize.contains("code_challenge_method=S256"));
    /// assert!(!authorize.contains(pkce.verifier()));
    /// // Keep state and verifier in the login session; verify returned state before exchanging code.
    /// ```
    pub fn new() -> Self {
        Self {
            verifier: format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
        }
    }
    /// Restore a login-session verifier after validating its 43 through 128 unreserved ASCII characters.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::oauth2::Pkce;
    /// let original = Pkce::new();
    /// let restored = Pkce::from_verifier(original.verifier())?;
    /// assert_eq!(restored.challenge(), original.challenge());
    /// # Ok::<(), rappin::YappinError>(())
    /// ```
    pub fn from_verifier(verifier: impl Into<String>) -> Result<Self> {
        let verifier = verifier.into();
        if !(43..=128).contains(&verifier.len())
            || !verifier
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-._~".contains(&c))
        {
            return Err(YappinError::validation(
                "code_verifier",
                "requires 43 to 128 unreserved ASCII characters",
            ));
        }
        Ok(Self { verifier })
    }
    /// Borrow the private verifier; do not include it in authorization URLs or logs.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::oauth2::{OAuth2Client, Pkce};
    /// let client = OAuth2Client::public("example-client-id", "http://localhost:3000/callback");
    /// let pkce = Pkce::new();
    /// let state = uuid::Uuid::new_v4().to_string();
    /// let authorize = client.authorization_url_pkce(&state, &pkce);
    /// assert!(authorize.contains("code_challenge_method=S256"));
    /// assert!(!authorize.contains(pkce.verifier()));
    /// // Keep state and verifier in the login session; verify returned state before exchanging code.
    /// ```
    pub fn verifier(&self) -> &str {
        &self.verifier
    }
    /// Return the S256 URL-safe challenge for the verifier.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::oauth2::{OAuth2Client, Pkce};
    /// let client = OAuth2Client::public("example-client-id", "http://localhost:3000/callback");
    /// let pkce = Pkce::new();
    /// let state = uuid::Uuid::new_v4().to_string();
    /// let authorize = client.authorization_url_pkce(&state, &pkce);
    /// assert!(authorize.contains("code_challenge_method=S256"));
    /// assert!(!authorize.contains(pkce.verifier()));
    /// // Keep state and verifier in the login session; verify returned state before exchanging code.
    /// ```
    pub fn challenge(&self) -> String {
        URL_SAFE_NO_PAD.encode(Sha256::digest(self.verifier.as_bytes()))
    }
}
#[derive(Clone)]
/// OAuth authorization-code client; user authorization tokens are separate from bot tokens.
///
/// # Example
///
/// ```rust
/// use rappin::oauth2::{OAuth2Client, Pkce};
/// let client = OAuth2Client::public("example-client-id", "http://localhost:3000/callback");
/// let pkce = Pkce::new();
/// let state = uuid::Uuid::new_v4().to_string();
/// let authorize = client.authorization_url_pkce(&state, &pkce);
/// assert!(authorize.contains("code_challenge_method=S256"));
/// assert!(!authorize.contains(pkce.verifier()));
/// // Keep state and verifier in the login session; verify returned state before exchanging code.
/// ```
pub struct OAuth2Client {
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    base_url: String,
    http: reqwest::Client,
}
impl OAuth2Client {
    /// Configure a confidential application. Keep the client secret on the server; use public plus PKCE for desktop/mobile apps.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::oauth2::OAuth2Client;
    /// fn configured(client_id: String, client_secret: String) -> OAuth2Client {
    ///     OAuth2Client::new(client_id, client_secret, "https://example.com/oauth/callback")
    /// }
    /// ```
    pub fn new(
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        redirect_uri: impl Into<String>,
    ) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            redirect_uri: redirect_uri.into(),
            base_url: DEFAULT_SITE_BASE.into(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("HTTP client"),
        }
    }
    /// Public desktop/mobile applications authenticate with PKCE and no secret.
    /// # Example
    ///
    /// ```rust
    /// use rappin::oauth2::{OAuth2Client, Pkce};
    /// let client = OAuth2Client::public("example-client-id", "http://localhost:3000/callback");
    /// let pkce = Pkce::new();
    /// let state = uuid::Uuid::new_v4().to_string();
    /// let authorize = client.authorization_url_pkce(&state, &pkce);
    /// assert!(authorize.contains("code_challenge_method=S256"));
    /// assert!(!authorize.contains(pkce.verifier()));
    /// // Keep state and verifier in the login session; verify returned state before exchanging code.
    /// ```
    pub fn public(client_id: impl Into<String>, redirect_uri: impl Into<String>) -> Self {
        Self::new(client_id, "", redirect_uri)
    }
    /// Override the website origin for an explicitly trusted host; redirects remain disabled.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::oauth2::{OAuth2Client, Pkce};
    /// let client = OAuth2Client::public("example-client-id", "http://localhost:3000/callback");
    /// let pkce = Pkce::new();
    /// let state = uuid::Uuid::new_v4().to_string();
    /// let authorize = client.authorization_url_pkce(&state, &pkce);
    /// assert!(authorize.contains("code_challenge_method=S256"));
    /// assert!(!authorize.contains(pkce.verifier()));
    /// // Keep state and verifier in the login session; verify returned state before exchanging code.
    /// let client = client.base_url("https://www.yapin.gg");
    /// ```
    pub fn base_url(mut self, base: impl Into<String>) -> Self {
        self.base_url = base.into().trim_end_matches('/').into();
        self
    }
    /// Yapin supports only the identify scope. Validate returned state against your login session.
    /// # Example
    ///
    /// ```rust
    /// use rappin::oauth2::OAuth2Client;
    /// let client = OAuth2Client::public("example-client-id", "http://localhost:3000/callback");
    /// let state = uuid::Uuid::new_v4().to_string();
    /// let url = client.authorization_url(&["identify"], Some(&state));
    /// assert!(url.contains("scope=identify"));
    /// // Public clients must use authorization_url_pkce for the actual login flow.
    /// ```
    pub fn authorization_url(&self, scopes: &[&str], state: Option<&str>) -> String {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", &self.redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", &scopes.join(" "));
        if let Some(state) = state {
            query.append_pair("state", state);
        }
        format!("{}/oauth/authorize?{}", self.base_url, query.finish())
    }
    /// Create an identify authorization URL with S256 PKCE and caller-owned CSRF state.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::oauth2::{OAuth2Client, Pkce};
    /// let client = OAuth2Client::public("example-client-id", "http://localhost:3000/callback");
    /// let pkce = Pkce::new();
    /// let state = uuid::Uuid::new_v4().to_string();
    /// let authorize = client.authorization_url_pkce(&state, &pkce);
    /// assert!(authorize.contains("code_challenge_method=S256"));
    /// assert!(!authorize.contains(pkce.verifier()));
    /// // Keep state and verifier in the login session; verify returned state before exchanging code.
    /// ```
    pub fn authorization_url_pkce(&self, state: &str, pkce: &Pkce) -> String {
        format!(
            "{}&code_challenge={}&code_challenge_method=S256",
            self.authorization_url(&["identify"], Some(state)),
            pkce.challenge()
        )
    }
    /// Exchange a confidential-app authorization code; the caller must verify callback state first.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{oauth2::OAuth2Client, Result};
    /// async fn complete(client: &OAuth2Client, code: &str) -> Result<()> {
    ///     let response = client.exchange_code(code).await?;
    ///     // Store credentials only in application-owned protected session storage.
    ///     Ok(())
    /// }
    /// ```
    pub async fn exchange_code(&self, code: &str) -> Result<OAuth2Token> {
        self.exchange(code, None).await
    }
    /// Exchange the code with the verifier bound to its login session; verify callback state first.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{oauth2::OAuth2Client, Result};
    /// async fn complete(client: &OAuth2Client, code: &str, pkce: &rappin::oauth2::Pkce) -> Result<()> {
    ///     let response = client.exchange_code_pkce(code, pkce).await?;
    ///     // Store credentials only in application-owned protected session storage.
    ///     Ok(())
    /// }
    /// ```
    pub async fn exchange_code_pkce(&self, code: &str, pkce: &Pkce) -> Result<OAuth2Token> {
        self.exchange(code, Some(pkce.verifier())).await
    }
    async fn exchange(&self, code: &str, verifier: Option<&str>) -> Result<OAuth2Token> {
        if self.client_secret.is_empty() && verifier.is_none() {
            return Err(YappinError::validation(
                "code_verifier",
                "public clients require PKCE",
            ));
        }
        let mut params = vec![
            ("client_id", self.client_id.as_str()),
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &self.redirect_uri),
        ];
        if !self.client_secret.is_empty() {
            params.push(("client_secret", &self.client_secret));
        }
        if let Some(v) = verifier {
            params.push(("code_verifier", v));
        }
        let res = self
            .http
            .post(crate::security::api_url(
                &self.base_url,
                "/api/oauth/token",
            )?)
            .form(&params)
            .send()
            .await
            .map_err(|e| YappinError::Http(e.without_url()))?;
        decode(res, &[&self.client_secret, code, verifier.unwrap_or("")]).await
    }
    /// Fetch /api/oauth/me using a user access token with identify scope, not a bot token.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{oauth2::OAuth2Client, Result};
    /// async fn complete(client: &OAuth2Client, access_token: &str) -> Result<()> {
    ///     let response = client.get_user_profile(access_token).await?;
    ///     // Store credentials only in application-owned protected session storage.
    ///     Ok(())
    /// }
    /// ```
    pub async fn get_user_profile(&self, access_token: &str) -> Result<OAuth2User> {
        let res = self
            .http
            .get(crate::security::api_url(&self.base_url, "/api/oauth/me")?)
            .header(
                header::AUTHORIZATION,
                crate::security::authorization("Bearer", access_token)?,
            )
            .send()
            .await
            .map_err(|e| YappinError::Http(e.without_url()))?;
        decode(res, &[access_token]).await
    }
}
async fn decode<T: serde::de::DeserializeOwned>(
    res: reqwest::Response,
    secrets: &[&str],
) -> Result<T> {
    let status = res.status();
    let delay = res
        .headers()
        .get("Retry-After")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v >= 0.0);
    let bytes = res.bytes().await?;
    if !status.is_success() {
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        let message: String = body["error_description"]
            .as_str()
            .or(body["error"].as_str())
            .unwrap_or("OAuth request failed")
            .into();
        return Err(YappinError::Api {
            status: status.as_u16(),
            message: crate::security::scrub(&message, secrets),
            retry_after: delay,
        });
    }
    serde_json::from_slice(&bytes).map_err(|e| crate::security::json_error(e, secrets))
}

impl std::fmt::Debug for OAuth2Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuth2Token")
            .field("access_token", &"[REDACTED]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("token_type", &self.token_type)
            .field("expires_in", &self.expires_in)
            .finish_non_exhaustive()
    }
}
