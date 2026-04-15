//! AuthProvider trait and Google OIDC implementation.

use std::future::Future;
use std::pin::Pin;

use super::AuthIdentity;
use anyhow::{anyhow, Result};

pub trait AuthProvider: Send + Sync {
    fn validate_token<'a>(
        &'a self,
        token: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<AuthIdentity>> + Send + 'a>>;
    fn provider_name(&self) -> &str;
    fn client_id(&self) -> Option<&str> {
        None
    }
}

// ---------------------------------------------------------------------------
// Google OIDC provider
// ---------------------------------------------------------------------------

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use std::sync::RwLock;

/// JWT claims we extract from Google ID tokens.
#[derive(Debug, Deserialize)]
struct GoogleClaims {
    email: String,
    email_verified: bool,
    name: String,
    picture: Option<String>,
    #[allow(dead_code)]
    hd: Option<String>,
    #[allow(dead_code)]
    aud: String,
}

/// A single JWK from Google's JWKS endpoint.
#[derive(Debug, Deserialize)]
struct Jwk {
    kid: String,
    n: String,
    e: String,
}

/// Wrapper for the JWKS response.
#[derive(Debug, Deserialize)]
struct JwksResponse {
    keys: Vec<Jwk>,
}

pub struct GoogleAuthProvider {
    client_id: String,
    http: reqwest::Client,
    /// Cached JWKS keys -- refreshed when a `kid` is not found.
    cached_keys: RwLock<Vec<Jwk>>,
}

impl GoogleAuthProvider {
    const JWKS_URL: &'static str = "https://www.googleapis.com/oauth2/v3/certs";

    pub fn new(client_id: String) -> Self {
        Self {
            client_id,
            http: reqwest::Client::new(),
            cached_keys: RwLock::new(Vec::new()),
        }
    }

    /// Fetch JWKS keys from Google and update the cache.
    async fn refresh_keys(&self) -> Result<()> {
        let resp: JwksResponse = self
            .http
            .get(Self::JWKS_URL)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let mut cache = self.cached_keys.write().map_err(|e| anyhow!("{e}"))?;
        *cache = resp.keys;
        Ok(())
    }

    /// Find a decoding key matching the given `kid`, refreshing once if not found.
    async fn decoding_key_for_kid(&self, kid: &str) -> Result<DecodingKey> {
        // First try the cache.
        if let Some(key) = self.find_key(kid)? {
            return Ok(key);
        }
        // Miss -- refresh and retry.
        self.refresh_keys().await?;
        self.find_key(kid)?
            .ok_or_else(|| anyhow!("no JWKS key matching kid `{kid}`"))
    }

    fn find_key(&self, kid: &str) -> Result<Option<DecodingKey>> {
        let cache = self.cached_keys.read().map_err(|e| anyhow!("{e}"))?;
        for jwk in cache.iter() {
            if jwk.kid == kid {
                let n_bytes = URL_SAFE_NO_PAD.decode(&jwk.n)?;
                let e_bytes = URL_SAFE_NO_PAD.decode(&jwk.e)?;
                return Ok(Some(DecodingKey::from_rsa_raw_components(
                    &n_bytes, &e_bytes,
                )));
            }
        }
        Ok(None)
    }
}

impl AuthProvider for GoogleAuthProvider {
    fn validate_token<'a>(
        &'a self,
        token: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<AuthIdentity>> + Send + 'a>> {
        Box::pin(async move {
            let header = decode_header(token)?;
            let kid = header
                .kid
                .ok_or_else(|| anyhow!("JWT header missing `kid`"))?;
            let key = self.decoding_key_for_kid(&kid).await?;

            let mut validation = Validation::new(Algorithm::RS256);
            validation.set_audience(&[&self.client_id]);
            validation.set_issuer(&["https://accounts.google.com", "accounts.google.com"]);

            let token_data = decode::<GoogleClaims>(token, &key, &validation)?;
            let claims = token_data.claims;

            if !claims.email_verified {
                return Err(anyhow!("Google account email not verified"));
            }

            Ok(AuthIdentity {
                email: claims.email,
                display_name: claims.name,
                avatar_url: claims.picture,
                provider: "google".into(),
            })
        })
    }

    fn provider_name(&self) -> &str {
        "google"
    }

    fn client_id(&self) -> Option<&str> {
        Some(&self.client_id)
    }
}

// ---------------------------------------------------------------------------
// Mock provider (test / test-auth only)
// ---------------------------------------------------------------------------

pub struct MockAuthProvider;

impl AuthProvider for MockAuthProvider {
    fn validate_token<'a>(
        &'a self,
        token: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<AuthIdentity>> + Send + 'a>> {
        Box::pin(async move {
            let rest = token
                .strip_prefix("mock:")
                .ok_or_else(|| anyhow!("MockAuthProvider: token must start with `mock:`"))?;
            let (email, name) = rest.split_once(':').ok_or_else(|| {
                anyhow!("MockAuthProvider: expected format `mock:<email>:<name>`")
            })?;

            if email.is_empty() || name.is_empty() {
                return Err(anyhow!(
                    "MockAuthProvider: email and name must be non-empty"
                ));
            }

            Ok(AuthIdentity {
                email: email.to_owned(),
                display_name: name.to_owned(),
                avatar_url: None,
                provider: "mock".into(),
            })
        })
    }

    fn provider_name(&self) -> &str {
        "mock"
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_provider_valid_token() {
        let provider = MockAuthProvider;
        let identity = provider
            .validate_token("mock:alice@co.com:Alice Smith")
            .await
            .unwrap();
        assert_eq!(identity.email, "alice@co.com");
        assert_eq!(identity.display_name, "Alice Smith");
        assert_eq!(identity.provider, "mock");
    }

    #[tokio::test]
    async fn mock_provider_invalid_token() {
        let provider = MockAuthProvider;
        assert!(provider.validate_token("garbage").await.is_err());
    }
}
