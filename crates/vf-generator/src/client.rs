//! Claude API client for code generation.
//!
//! Provides a simple interface to the Anthropic Claude API.

use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Claude API configuration.
#[derive(Debug, Clone)]
pub struct ClaudeConfig {
    /// API key (from ANTHROPIC_API_KEY env var or explicit)
    pub api_key: String,
    /// Model to use
    pub model: String,
    /// Maximum tokens in response
    pub max_tokens: u32,
    /// Temperature (0.0 = deterministic, 1.0 = creative)
    pub temperature: f32,
    /// Request timeout
    pub timeout: Duration,
    /// API base URL
    pub base_url: String,
}

impl Default for ClaudeConfig {
    fn default() -> Self {
        Self {
            api_key: std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
            model: "claude-opus-4-5-20251101".to_string(),
            max_tokens: 16384,
            temperature: 0.0, // Deterministic for code generation
            timeout: Duration::from_secs(120),
            base_url: "https://api.anthropic.com".to_string(),
        }
    }
}

impl ClaudeConfig {
    /// Create config with API key from environment.
    pub fn from_env() -> Result<Self, ClientError> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| ClientError::MissingApiKey)?;

        Ok(Self {
            api_key,
            ..Default::default()
        })
    }

    /// Set the model.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Set max tokens.
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    /// Set temperature.
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        debug_assert!(
            (0.0..=1.0).contains(&temperature),
            "Temperature must be between 0.0 and 1.0"
        );
        self.temperature = temperature;
        self
    }
}

/// Message role in conversation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// A message in the conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    /// Create a user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    /// Create an assistant message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// Claude API client.
pub struct ClaudeClient {
    config: ClaudeConfig,
    client: Client,
}

impl ClaudeClient {
    /// Create a new client with the given config.
    pub fn new(config: ClaudeConfig) -> Result<Self, ClientError> {
        if config.api_key.is_empty() {
            return Err(ClientError::MissingApiKey);
        }

        let client = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| ClientError::HttpError(e.to_string()))?;

        Ok(Self { config, client })
    }

    /// Create a client from environment variables.
    pub fn from_env() -> Result<Self, ClientError> {
        Self::new(ClaudeConfig::from_env()?)
    }

    /// Send a completion request.
    pub async fn complete(&self, messages: Vec<Message>) -> Result<String, ClientError> {
        self.complete_with_system(messages, None).await
    }

    /// Send a completion request with a system prompt (uses streaming).
    pub async fn complete_with_system(
        &self,
        messages: Vec<Message>,
        system: Option<String>,
    ) -> Result<String, ClientError> {
        debug_assert!(!messages.is_empty(), "Messages cannot be empty");

        let request = ApiRequest {
            model: &self.config.model,
            max_tokens: self.config.max_tokens,
            temperature: self.config.temperature,
            system: system.as_deref(),
            messages: &messages,
            stream: true,
        };

        let response = self
            .client
            .post(format!("{}/v1/messages", self.config.base_url))
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| ClientError::HttpError(e.to_string()))?;

        let status = response.status();

        if !status.is_success() {
            let body = response
                .text()
                .await
                .map_err(|e| ClientError::HttpError(e.to_string()))?;
            return Err(ClientError::ApiError {
                status: status.as_u16(),
                body,
            });
        }

        // Stream the response and accumulate text
        let mut accumulated_text = String::new();
        let body = response
            .text()
            .await
            .map_err(|e| ClientError::HttpError(e.to_string()))?;

        // Parse SSE events from the response
        for line in body.lines() {
            if let Some(data) = line.strip_prefix("data: ") {
                if data == "[DONE]" {
                    break;
                }

                // Parse the JSON event
                if let Ok(event) = serde_json::from_str::<StreamEvent>(data) {
                    if event.event_type == "content_block_delta" {
                        if let Some(delta) = event.delta {
                            if delta.delta_type == "text_delta" {
                                accumulated_text.push_str(&delta.text);
                            }
                        }
                    }
                }
            }
        }

        Ok(accumulated_text)
    }

    /// Get the model being used.
    pub fn model(&self) -> &str {
        &self.config.model
    }
}

/// API request body.
#[derive(Serialize)]
struct ApiRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<&'a str>,
    messages: &'a [Message],
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
}

/// API response body.
#[derive(Deserialize)]
struct ApiResponse {
    content: Vec<ContentBlock>,
    #[allow(dead_code)]
    model: String,
    #[allow(dead_code)]
    stop_reason: Option<String>,
}

/// Content block in response.
#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    content_type: String,
    #[serde(default)]
    text: String,
}

/// Streaming event from SSE response.
#[derive(Deserialize)]
struct StreamEvent {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    delta: Option<StreamDelta>,
}

/// Delta content in streaming response.
#[derive(Deserialize)]
struct StreamDelta {
    #[serde(rename = "type")]
    delta_type: String,
    #[serde(default)]
    text: String,
}

/// Client errors.
#[derive(Debug, Error)]
pub enum ClientError {
    #[error("ANTHROPIC_API_KEY environment variable not set")]
    MissingApiKey,

    #[error("HTTP error: {0}")]
    HttpError(String),

    #[error("API error (status {status}): {body}")]
    ApiError { status: u16, body: String },

    #[error("Parse error: {0}")]
    ParseError(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = ClaudeConfig::default();
        assert_eq!(config.model, "claude-opus-4-5-20251101");
        assert_eq!(config.temperature, 0.0);
    }

    #[test]
    fn test_message_creation() {
        let user = Message::user("Hello");
        assert_eq!(user.role, Role::User);
        assert_eq!(user.content, "Hello");

        let assistant = Message::assistant("Hi there");
        assert_eq!(assistant.role, Role::Assistant);
    }

    #[test]
    fn test_config_builder() {
        let config = ClaudeConfig::default()
            .with_model("claude-3-opus-20240229")
            .with_max_tokens(8192)
            .with_temperature(0.5);

        assert_eq!(config.model, "claude-3-opus-20240229");
        assert_eq!(config.max_tokens, 8192);
        assert_eq!(config.temperature, 0.5);
    }
}
