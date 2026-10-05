//! DeepSeek Chat Completions API (OpenAI-compatible) over plain HTTP.

use std::fmt;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Map, Value, json};

pub const API_URL: &str = "https://api.deepseek.com/chat/completions";

#[derive(Debug, Deserialize)]
pub struct ChatResponse {
    pub model: String,
    pub choices: Vec<Choice>,
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Deserialize)]
pub struct Choice {
    /// Kept as raw JSON so the whole assistant message (content,
    /// reasoning_content, tool_calls) can be sent back unchanged.
    pub message: Value,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// DeepSeek context caching: the part of `prompt_tokens` served from cache.
    #[serde(default)]
    pub prompt_cache_hit_tokens: Option<u64>,
}

/// One request/response pair. Written to the raw log without auth headers.
pub struct Exchange {
    pub request: Value,
    pub response: ChatResponse,
    pub response_json: Value,
}

#[derive(Debug)]
pub enum ApiError {
    Timeout(String),
    Http(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::Timeout(msg) => write!(f, "timeout: {msg}"),
            ApiError::Http(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for ApiError {}

pub struct Client {
    http: reqwest::blocking::Client,
    endpoint: String,
    api_key: String,
    model: String,
    max_tokens: u32,
    params: Map<String, Value>,
}

impl Client {
    pub fn new(
        api_key: String,
        model: &str,
        max_tokens: u32,
        params: &Value,
        timeout: Duration,
    ) -> Result<Self, reqwest::Error> {
        // reqwest's blocking client times out after 30s by default; use the run budget instead.
        let http = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .build()?;
        let params = params.as_object().cloned().unwrap_or_default();
        // The mock endpoint exists only in the test binary, never in the shipped CLI.
        #[cfg(test)]
        let endpoint = std::env::var("HEL_TEST_API_URL").unwrap_or_else(|_| API_URL.to_string());
        #[cfg(not(test))]
        let endpoint = API_URL.to_string();
        Ok(Self {
            http,
            endpoint,
            api_key,
            model: model.to_string(),
            max_tokens,
            params,
        })
    }

    pub fn complete(
        &self,
        messages: &[Value],
        tools: Option<&Value>,
    ) -> Result<Exchange, ApiError> {
        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "max_tokens": self.max_tokens,
        });
        if let Some(tools) = tools {
            body["tools"] = tools.clone();
        }
        // Model params from the run context (e.g. `thinking`) are passed through as-is.
        for (key, value) in &self.params {
            body[key] = value.clone();
        }

        let response = self
            .http
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .map_err(classify)?;

        let status = response.status();
        let text = response.text().map_err(classify)?;
        if !status.is_success() {
            return Err(ApiError::Http(format!(
                "HTTP {status}: {}",
                truncate(&text, 500)
            )));
        }

        let response_json: Value = serde_json::from_str(&text)
            .map_err(|e| ApiError::Http(format!("invalid JSON response: {e}")))?;
        let parsed: ChatResponse = serde_json::from_value(response_json.clone())
            .map_err(|e| ApiError::Http(format!("unexpected response shape: {e}")))?;

        Ok(Exchange {
            request: body,
            response: parsed,
            response_json,
        })
    }
}

fn classify(err: reqwest::Error) -> ApiError {
    if err.is_timeout() {
        ApiError::Timeout(err.to_string())
    } else {
        ApiError::Http(err.to_string())
    }
}

fn truncate(text: &str, max: usize) -> &str {
    match text.char_indices().nth(max) {
        Some((idx, _)) => &text[..idx],
        None => text,
    }
}
