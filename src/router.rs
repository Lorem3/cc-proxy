use crate::provider::{find_model_mapping, load_model_mapping, PlatformConfig};
use anyhow::{Context, Result};
use async_compression::tokio::bufread::GzipDecoder;
use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, Response, StatusCode},
};
use bytes::Bytes;
use futures::TryStreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::io::StreamReader;

#[derive(Clone)]
pub struct Router {
    http_client: reqwest::Client,
    model_mapping: Arc<RwLock<HashMap<String, PlatformConfig>>>,
    request_log: bool,
}

fn format_body_for_log(body: &Bytes) -> String {
    if let Ok(json) = serde_json::from_slice::<Value>(body) {
        serde_json::to_string_pretty(&json)
            .unwrap_or_else(|_| String::from_utf8_lossy(body).into_owned())
    } else {
        String::from_utf8_lossy(body).into_owned()
    }
}

impl Router {
    pub fn new(http_client: reqwest::Client, request_log: bool) -> Result<Self> {
        let mapping = match load_model_mapping() {
            Ok(m) => {
                tracing::info!("Loaded {} model mapping(s)", m.len());
                m
            }
            Err(e) => {
                tracing::warn!("Failed to load model_mapping: {}", e);
                HashMap::new()
            }
        };

        Ok(Self {
            http_client,
            model_mapping: Arc::new(RwLock::new(mapping)),
            request_log,
        })
    }

    /// Reload model_mapping from disk
    pub async fn reload_config(&self) -> Result<()> {
        tracing::info!("Reloading model_mapping from config file");

        let mapping = load_model_mapping()?;
        let mapping_count = mapping.len();
        let mut mm = self.model_mapping.write().await;
        *mm = mapping;

        tracing::info!("✓ Reloaded {} model mapping(s)", mapping_count);
        Ok(())
    }

    /// Route a request using model_mapping only.
    pub async fn route_request(
        &self,
        kind: &str,
        incoming_url: &str,
        endpoint: &str,
        body: Bytes,
        headers: HeaderMap,
    ) -> Result<Response<Body>> {
        let original_body = body.clone();
        let request_json: Value =
            serde_json::from_slice(&body).context("Failed to parse request body as JSON")?;

        let model = request_json["model"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();

        tracing::debug!("Request: kind={}, model={}", kind, model);

        let mapping = self.model_mapping.read().await;
        if mapping.is_empty() {
            anyhow::bail!("No model_mapping configured");
        }

        let Some((key, cfg)) = find_model_mapping(&mapping, &model) else {
            anyhow::bail!("Model '{}' not found in model_mapping", model);
        };
        let cfg = cfg.clone();
        drop(mapping);

        let forward_body = if let Some(name) = cfg.name.as_ref().filter(|n| !n.is_empty()) {
            tracing::info!(
                "model_mapping hit: model={} → key={} url={} name={}",
                model,
                key,
                cfg.api_url,
                name
            );
            let mut modified = request_json;
            modified["model"] = Value::String(name.clone());
            Bytes::from(serde_json::to_vec(&modified)?)
        } else {
            tracing::info!(
                "model_mapping hit: model={} → key={} url={}",
                model,
                key,
                cfg.api_url
            );
            body
        };

        self.forward_request(
            &cfg,
            incoming_url,
            endpoint,
            &original_body,
            &forward_body,
            &headers,
        )
        .await
    }

    /// Forward request to the mapped upstream.
    async fn forward_request(
        &self,
        cfg: &PlatformConfig,
        incoming_url: &str,
        endpoint: &str,
        original_body: &Bytes,
        body: &Bytes,
        headers: &HeaderMap,
    ) -> Result<Response<Body>> {
        let url = format!("{}{}", cfg.api_url.trim_end_matches('/'), endpoint);

        if self.request_log {
            tracing::info!(
                target: "cc_mapping::request",
                incoming_url = %incoming_url,
                "proxy request"
            );
            tracing::info!(
                target: "cc_mapping::request",
                upstream_url = %url,
                "proxy request"
            );
            tracing::info!(
                target: "cc_mapping::request",
                body = %format_body_for_log(original_body),
                "original body"
            );
            tracing::info!(
                target: "cc_mapping::request",
                body = %format_body_for_log(body),
                "forward body"
            );
        }

        let mut req_headers = reqwest::header::HeaderMap::new();
        for (key, value) in headers {
            if key == "host" || key == "authorization" {
                continue;
            }

            let lower = key.as_str().to_ascii_lowercase();
            let is_hop_by_hop = matches!(
                lower.as_str(),
                "connection"
                    | "proxy-connection"
                    | "keep-alive"
                    | "transfer-encoding"
                    | "upgrade"
                    | "te"
                    | "trailers"
            );
            if is_hop_by_hop || lower == "content-length" {
                continue;
            }

            if let Ok(req_name) = reqwest::header::HeaderName::from_bytes(key.as_str().as_bytes()) {
                if let Ok(val) = reqwest::header::HeaderValue::from_bytes(value.as_bytes()) {
                    req_headers.insert(req_name, val);
                }
            }
        }

        req_headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", cfg.api_key))?,
        );

        if !req_headers.contains_key(reqwest::header::ACCEPT) {
            req_headers.insert(
                reqwest::header::ACCEPT,
                reqwest::header::HeaderValue::from_static("application/json"),
            );
        }

        let response = self
            .http_client
            .post(&url)
            .headers(req_headers)
            .body(body.to_vec())
            .send()
            .await
            .context("Failed to send request to upstream")?;

        let status = response.status();

        if !status.is_success() {
            anyhow::bail!("Upstream returned error status: {}", status);
        }

        if let Some(tengine_error) = response.headers().get("x-tengine-error") {
            match tengine_error.to_str() {
                Ok(err) => tracing::warn!(
                    "Upstream indicated potential WAF block via x-tengine-error={}, forwarding anyway",
                    err
                ),
                Err(_) => tracing::warn!(
                    "Upstream indicated potential WAF block via x-tengine-error (non-UTF8), forwarding anyway"
                ),
            }
        }

        if let Some(content_type) = response.headers().get("content-type") {
            match content_type.to_str() {
                Ok(ct_str)
                    if !ct_str.contains("application/json")
                        && !ct_str.contains("text/event-stream") =>
                {
                    tracing::warn!(
                        "Upstream returned unexpected content-type '{}', forwarding response anyway",
                        ct_str
                    );
                }
                Err(_) => tracing::warn!(
                    "Upstream returned non-UTF8 content-type header, forwarding response anyway"
                ),
                _ => {}
            }
        }

        let axum_status = StatusCode::from_u16(status.as_u16())?;
        let mut axum_response = Response::builder().status(axum_status);

        let mut has_gzip_encoding = false;
        for (key, value) in response.headers() {
            let key_str = key.as_str();
            let is_hop_by_hop = matches!(
                key_str.to_ascii_lowercase().as_str(),
                "connection"
                    | "proxy-connection"
                    | "keep-alive"
                    | "transfer-encoding"
                    | "upgrade"
                    | "te"
                    | "trailers"
            );

            if key_str.eq_ignore_ascii_case("content-encoding") {
                if let Ok(val_str) = value.to_str() {
                    if val_str.eq_ignore_ascii_case("gzip") {
                        has_gzip_encoding = true;
                        tracing::debug!("Response is gzip-encoded, will decompress");
                        continue;
                    }
                }
            }

            if is_hop_by_hop || key_str.eq_ignore_ascii_case("content-length") {
                tracing::debug!("Skipping header: {}", key_str);
                continue;
            }

            if let Ok(val) = HeaderValue::from_bytes(value.as_bytes()) {
                tracing::debug!("Forwarding header: {}: {:?}", key_str, val);
                axum_response = axum_response.header(key_str, val);
            }
        }

        let stream = response.bytes_stream().map_err(std::io::Error::other);

        let body = if has_gzip_encoding {
            tracing::debug!("Decompressing gzipped response");
            let reader = StreamReader::new(stream);
            let decoder = GzipDecoder::new(reader);
            let decompressed_stream =
                tokio_util::io::ReaderStream::new(decoder).map_err(std::io::Error::other);
            Body::from_stream(decompressed_stream)
        } else {
            Body::from_stream(stream.inspect_ok(|chunk| {
                if !chunk.is_empty() {
                    let preview = &chunk[..chunk.len().min(50)];
                    match std::str::from_utf8(preview) {
                        Ok(s) => tracing::debug!("Response chunk (UTF-8): {:?}...", s),
                        Err(_) => tracing::debug!(
                            "Response chunk (bytes): {:02x?}...",
                            &preview[..preview.len().min(20)]
                        ),
                    }
                }
            }))
        };

        axum_response.body(body).context("Failed to build response")
    }
}

/// Create error response
pub fn error_response(status: StatusCode, message: &str) -> Response<Body> {
    let error_json = serde_json::json!({
        "error": message
    });

    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(error_json.to_string()))
        .unwrap()
}
