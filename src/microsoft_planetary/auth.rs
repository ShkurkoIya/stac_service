use chrono::{DateTime, Duration, Utc};
use http::Extensions;
use reqwest::{Client, Request, Response};
use reqwest_middleware::{Middleware, Next, Result as MiddlewareResult};
use serde::Deserialize;
use std::collections::HashMap;
use tokio::sync::RwLock;

#[derive(Deserialize)]
struct SasTokenResponse {
    token: String,
    #[serde(rename = "msft:expiry")]
    expiry: DateTime<Utc>,
}

struct CachedToken {
    token: String,
    expires_at: DateTime<Utc>,
}

pub struct MicrosoftSasMiddleware {
    auth_client: Client,
    cache: RwLock<HashMap<String, CachedToken>>,
}

impl Default for MicrosoftSasMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

impl MicrosoftSasMiddleware {
    pub fn new() -> Self {
        Self {
            auth_client: Client::new(),
            cache: RwLock::new(HashMap::new()),
        }
    }

    async fn get_token(&self, account: &str, container: &str) -> Option<String> {
        let cache_key = format!("{}/{}", account, container);

        {
            let read_guard = self.cache.read().await;
            if let Some(cached) = read_guard.get(&cache_key)
                && Utc::now() + Duration::minutes(5) < cached.expires_at
            {
                return Some(cached.token.clone());
            }
        }

        // Обновляем токен через SAS api Microsoft
        let mut write_guard = self.cache.write().await;
        if let Some(cached) = write_guard.get(&cache_key)
            && Utc::now() + Duration::minutes(5) < cached.expires_at
        {
            return Some(cached.token.clone());
        }

        let sign_url = format!(
            "https://planetarycomputer.microsoft.com/api/sas/v1/token/{}/{}",
            account, container
        );

        let resp = self.auth_client.get(&sign_url).send().await.ok()?;
        let data: SasTokenResponse = resp.json().await.ok()?;

        println!("🔑 [Middleware] Выдан SAS-токен для Azure: {}", cache_key);

        let token = data.token.clone();

        write_guard.insert(
            cache_key,
            CachedToken {
                token: data.token,
                expires_at: data.expiry,
            },
        );

        Some(token)
    }
}

#[async_trait::async_trait]
impl Middleware for MicrosoftSasMiddleware {
    async fn handle(
        &self,
        mut req: Request,
        extensions: &mut Extensions,
        next: Next<'_>,
    ) -> MiddlewareResult<Response> {
        let url = req.url().clone();

        if let Some(host) = url.host_str()
            && host.ends_with(".blob.core.windows.net")
        {
            // Вытаскиваем account из поддомена (например, "landsateuwest")
            let account = host.split('.').next().unwrap_or_default();
            // Вытаскиваем container из первого сегмента пути (например, "landsat-c2")
            let container = url
                .path_segments()
                .and_then(|mut s| s.next())
                .unwrap_or_default();

            if !account.is_empty()
                && !container.is_empty()
                && let Some(token) = self.get_token(account, container).await
            {
                // Аккуратно дописываем query-параметры токена прямо в URL запроса
                let mut new_url = url.clone();
                let current_query = new_url.query().unwrap_or("");
                let updated_query = if current_query.is_empty() {
                    token
                } else {
                    format!("{}&{}", current_query, token)
                };
                new_url.set_query(Some(&updated_query));
                *req.url_mut() = new_url;
            }
        }

        next.run(req, extensions).await
    }
}
