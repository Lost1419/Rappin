//! OAuth2 authorization-code and S256 PKCE support for Login with Yapin'.
use crate::utils::DEFAULT_SITE_BASE;
use crate::{Result, YappinError};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::header;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuth2Token {
    pub access_token: String,
    #[serde(default)]
    pub token_type: String,
    #[serde(default)]
    pub expires_in: Option<u64>,
    /// Reserved for API evolution; Yapin currently does not issue refresh tokens.
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}
/// Identity returned directly by /api/oauth/me.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuth2User {
    pub id: uuid::Uuid,
    pub username: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}
/// Keep the verifier private in the login session and send only its challenge to authorize.
#[derive(Clone)]
pub struct Pkce {
    verifier: String,
}
impl Default for Pkce {
    fn default() -> Self {
        Self::new()
    }
}
impl Pkce {
    pub fn new() -> Self {
        Self {
            verifier: format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
        }
    }
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
    pub fn verifier(&self) -> &str {
        &self.verifier
    }
    pub fn challenge(&self) -> String {
        URL_SAFE_NO_PAD.encode(Sha256::digest(self.verifier.as_bytes()))
    }
}
#[derive(Clone)]
pub struct OAuth2Client {
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    base_url: String,
    http: reqwest::Client,
}
impl OAuth2Client {
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
                .build()
                .expect("HTTP client"),
        }
    }
    /// Public desktop/mobile applications authenticate with PKCE and no secret.
    pub fn public(client_id: impl Into<String>, redirect_uri: impl Into<String>) -> Self {
        Self::new(client_id, "", redirect_uri)
    }
    pub fn base_url(mut self, base: impl Into<String>) -> Self {
        self.base_url = base.into().trim_end_matches('/').into();
        self
    }
    /// Yapin supports only the identify scope. Validate returned state against your login session.
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
    pub fn authorization_url_pkce(&self, state: &str, pkce: &Pkce) -> String {
        format!(
            "{}&code_challenge={}&code_challenge_method=S256",
            self.authorization_url(&["identify"], Some(state)),
            pkce.challenge()
        )
    }
    pub async fn exchange_code(&self, code: &str) -> Result<OAuth2Token> {
        self.exchange(code, None).await
    }
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
            .post(format!("{}/api/oauth/token", self.base_url))
            .form(&params)
            .send()
            .await?;
        decode(res).await
    }
    pub async fn get_user_profile(&self, access_token: &str) -> Result<OAuth2User> {
        let res = self
            .http
            .get(format!("{}/api/oauth/me", self.base_url))
            .header(header::AUTHORIZATION, format!("Bearer {access_token}"))
            .send()
            .await?;
        decode(res).await
    }
}
async fn decode<T: serde::de::DeserializeOwned>(res: reqwest::Response) -> Result<T> {
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
        let message = body["error_description"]
            .as_str()
            .or(body["error"].as_str())
            .unwrap_or("OAuth request failed")
            .into();
        return Err(YappinError::Api {
            status: status.as_u16(),
            message,
            retry_after: delay,
        });
    }
    Ok(serde_json::from_slice(&bytes)?)
}
