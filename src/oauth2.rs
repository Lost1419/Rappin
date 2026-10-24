use std::time::Duration;

use reqwest::header;
use serde::{Deserialize, Serialize};

use crate::error::{Result, YappinError};
use crate::models::UserDetail;
use crate::utils::DEFAULT_SITE_BASE;

/// Token response returned after exchanging an authorization code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuth2Token {
    pub access_token: String,
    #[serde(default)]
    pub token_type: String,
    #[serde(default)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

/// Helper client to implement "Login with Yapin'" OAuth2 flows.
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
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .expect("failed to construct reqwest client");

        Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            redirect_uri: redirect_uri.into(),
            base_url: DEFAULT_SITE_BASE.to_string(),
            http,
        }
    }

    /// Generates the authorization URL to redirect users to Yapin' for login.
    pub fn authorization_url(&self, scopes: &[&str], state: Option<&str>) -> String {
        let scope_str = scopes.join(" ");
        let mut url = format!(
            "{}/oauth/authorize?client_id={}&redirect_uri={}&response_type=code&scope={}",
            self.base_url,
            url::form_urlencoded::byte_serialize(self.client_id.as_bytes()).collect::<String>(),
            url::form_urlencoded::byte_serialize(self.redirect_uri.as_bytes()).collect::<String>(),
            url::form_urlencoded::byte_serialize(scope_str.as_bytes()).collect::<String>(),
        );

        if let Some(s) = state {
            url.push_str(&format!(
                "&state={}",
                url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>()
            ));
        }

        url
    }

    /// Exchanges the authorization code received in the redirect for an access token.
    pub async fn exchange_code(&self, code: &str) -> Result<OAuth2Token> {
        let token_url = format!("{}/api/oauth/token", self.base_url);

        let params = [
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.as_str()),
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", self.redirect_uri.as_str()),
        ];

        let res = self
            .http
            .post(&token_url)
            .form(&params)
            .header(header::USER_AGENT, "rappin-oauth/0.1.0")
            .send()
            .await?;

        let status = res.status();
        if !status.is_success() {
            let body = res.text().await.unwrap_or_default();
            return Err(YappinError::Api {
                status: status.as_u16(),
                message: body,
                retry_after: None,
            });
        }

        let token: OAuth2Token = res.json().await?;
        Ok(token)
    }

    /// Fetches the user profile associated with an access token.
    pub async fn get_user_profile(&self, access_token: &str) -> Result<UserDetail> {
        let user_url = format!("{}/api/oauth/user", self.base_url);

        let res = self
            .http
            .get(&user_url)
            .header(header::AUTHORIZATION, format!("Bearer {}", access_token))
            .header(header::USER_AGENT, "rappin-oauth/0.1.0")
            .send()
            .await?;

        let status = res.status();
        if !status.is_success() {
            let body = res.text().await.unwrap_or_default();
            return Err(YappinError::Api {
                status: status.as_u16(),
                message: body,
                retry_after: None,
            });
        }

        #[derive(Deserialize)]
        struct Resp {
            user: UserDetail,
        }

        let parsed: Resp = res.json().await?;
        Ok(parsed.user)
    }
}
