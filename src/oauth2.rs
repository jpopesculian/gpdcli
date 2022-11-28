use eyre::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";

lazy_static::lazy_static! {
    static ref OAUTH2_CLIENT: reqwest::Client = reqwest::Client::new();
}

#[derive(Clone, Debug, Deserialize)]
pub struct ServiceAccount {
    // project_id: String,
    // private_key_id: String,
    private_key: String,
    client_email: String,
    // client_id: String,
    // auth_uri: String,
    // token_uri: String,
    // auth_provider_x509_cert_url: String,
    // client_x509_cert_url: String,
}

#[derive(Clone, Debug, Serialize)]
struct Jwt {
    iss: String,
    scope: String,
    aud: String,
    iat: u64,
    exp: u64,
}

impl Jwt {
    fn new(service_account: &ServiceAccount, scope: String) -> Self {
        let iat = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        Self {
            scope,
            iss: service_account.client_email.clone(),
            aud: TOKEN_ENDPOINT.to_owned(),
            iat: iat.as_secs(),
            exp: (iat + Duration::from_secs(3600)).as_secs(),
        }
    }

    fn encode(&self, service_account: &ServiceAccount) -> String {
        jsonwebtoken::encode(
            &jsonwebtoken::Header {
                alg: jsonwebtoken::Algorithm::RS256,
                typ: Some("JWT".into()),
                ..Default::default()
            },
            &self,
            &jsonwebtoken::EncodingKey::from_rsa_pem(service_account.private_key.as_ref()).unwrap(),
        )
        .unwrap()
    }
}

#[derive(Debug, Clone, Serialize)]
struct TokenRequest {
    grant_type: String,
    assertion: String,
}

impl TokenRequest {
    fn build(service_account: &ServiceAccount, jwt: &Jwt) -> Self {
        TokenRequest {
            grant_type: "urn:ietf:params:oauth:grant-type:jwt-bearer".to_owned(),
            assertion: jwt.encode(service_account),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: i64,
}

#[derive(Debug, Clone)]
pub struct Oauth2Token {
    pub access_token: String,
    pub expires_at: time::OffsetDateTime,
}

pub struct Oauth2TokenManager {
    service_account: ServiceAccount,
    scope: String,
    token: Arc<Mutex<Option<Oauth2Token>>>,
}

impl Oauth2TokenManager {
    pub fn new<'a>(service_account: ServiceAccount, scopes: impl AsRef<[&'a str]>) -> Self {
        Self {
            service_account,
            scope: scopes.as_ref().join(","),
            token: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn token(&self) -> Result<Oauth2Token> {
        let mut token = self.token.lock().await;
        if let Some(token) = token.as_ref() {
            if (token.expires_at - time::OffsetDateTime::now_utc()) > time::Duration::minutes(5) {
                return Ok(token.clone());
            }
        }
        let new_token = self.request_access_token().await?;
        *token = Some(new_token.clone());
        Ok(new_token)
    }

    async fn request_access_token<'a>(&self) -> Result<Oauth2Token> {
        let token_res: TokenResponse = OAUTH2_CLIENT
            .post(TOKEN_ENDPOINT)
            .form(&TokenRequest::build(
                &self.service_account,
                &Jwt::new(&self.service_account, self.scope.clone()),
            ))
            .send()
            .await?
            .json()
            .await?;
        Ok(Oauth2Token {
            access_token: token_res.access_token.trim_end_matches('.').to_string(),
            expires_at: time::OffsetDateTime::now_utc()
                + time::Duration::seconds(token_res.expires_in),
        })
    }
}
