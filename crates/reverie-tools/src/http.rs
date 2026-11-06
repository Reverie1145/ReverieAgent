//! Generic REST/GraphQL client for external APIs.

use std::time::Duration;

use async_trait::async_trait;
use reverie_core::domain::ToolOutcome;
use serde_json::{json, Value};

use crate::args::{opt_i64_clamped, opt_object, opt_str, req_str};
use crate::context::ToolExecutionContext;
use crate::error::{ToolError, ToolResult};
use crate::registry::{AgentTool, ToolCategory};

const DEFAULT_TIMEOUT_MS: i64 = 60_000;
const MAX_BODY_CHARS: usize = 100_000;

/// Perform an arbitrary HTTP request.
pub struct HttpApiRequestTool;

fn parse_method(raw: &str) -> ToolResult<reqwest::Method> {
    match raw.trim().to_ascii_uppercase().as_str() {
        "GET" => Ok(reqwest::Method::GET),
        "POST" => Ok(reqwest::Method::POST),
        "PUT" => Ok(reqwest::Method::PUT),
        "PATCH" => Ok(reqwest::Method::PATCH),
        "DELETE" => Ok(reqwest::Method::DELETE),
        "HEAD" => Ok(reqwest::Method::HEAD),
        other => Err(ToolError::invalid(format!("unsupported HTTP method `{other}`"))),
    }
}

#[async_trait]
impl AgentTool for HttpApiRequestTool {
    fn name(&self) -> &str {
        "http_api_request"
    }

    fn description(&self) -> &str {
        "Call an external REST or GraphQL API. Supports custom headers, a raw string body or a JSON \
         body, and returns the status code, response headers and body."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "Absolute http(s) URL"},
                "method": {
                    "type": "string",
                    "enum": ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"],
                    "default": "GET"
                },
                "headers": {"type": "object", "description": "Additional request headers"},
                "json": {"type": "object", "description": "JSON body (sets Content-Type)"},
                "body": {"type": "string", "description": "Raw body, used when `json` is absent"},
                "timeout_ms": {"type": "integer", "description": "Request timeout", "default": DEFAULT_TIMEOUT_MS}
            },
            "required": ["url"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Network
    }

    async fn execute(&self, ctx: &ToolExecutionContext, args: Value) -> ToolResult<ToolOutcome> {
        let url = req_str(&args, "url")?;
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(ToolError::invalid("url must start with http:// or https://"));
        }
        let method = parse_method(&opt_str(&args, "method").unwrap_or_else(|| "GET".into()))?;
        let timeout_ms = opt_i64_clamped(&args, "timeout_ms", DEFAULT_TIMEOUT_MS, 1_000, 300_000);
        let headers = opt_object(&args, "headers");
        let json_body = args.get("json").filter(|value| value.is_object()).cloned();
        let raw_body = opt_str(&args, "body");

        ctx.emit_activity(format!("{method} {}", reverie_core::util::truncate_chars(&url, 70)));

        let mut request = ctx
            .http
            .request(method.clone(), &url)
            .timeout(Duration::from_millis(timeout_ms as u64));

        for (key, value) in headers {
            let rendered = match value {
                Value::String(text) => text,
                other => other.to_string(),
            };
            request = request.header(key, rendered);
        }
        if let Some(body) = json_body {
            request = request.json(&body);
        } else if let Some(body) = raw_body {
            request = request.body(body);
        }

        let response = request.send().await?;
        let status = response.status();
        let response_headers: Vec<String> = response
            .headers()
            .iter()
            .map(|(key, value)| format!("{key}: {}", value.to_str().unwrap_or("<binary>")))
            .collect();
        let body = response.text().await.unwrap_or_default();
        let body_preview = reverie_core::util::truncate_chars(&body, MAX_BODY_CHARS);

        let mut rendered = format!("{} {url}\nstatus: {}\n", method, status.as_u16());
        rendered.push_str("\n--- headers ---\n");
        rendered.push_str(&response_headers.join("\n"));
        rendered.push_str("\n\n--- body ---\n");
        rendered.push_str(&body_preview);

        let outcome = ToolOutcome::ok_with(
            ctx.cap_output(rendered),
            json!({"status": status.as_u16(), "body_bytes": body.len()}),
        );
        if !status.is_success() {
            return Ok(ToolOutcome { content: outcome.content, is_error: true, metadata: outcome.metadata });
        }
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_parsing_is_case_insensitive() {
        assert_eq!(parse_method("post").unwrap(), reqwest::Method::POST);
        assert_eq!(parse_method(" Delete ").unwrap(), reqwest::Method::DELETE);
        assert!(parse_method("TRACE").is_err());
    }

    #[test]
    fn schema_lists_methods() {
        let params = HttpApiRequestTool.parameters();
        assert_eq!(params["properties"]["method"]["enum"].as_array().unwrap().len(), 6);
        assert_eq!(params["required"][0], "url");
    }
}
