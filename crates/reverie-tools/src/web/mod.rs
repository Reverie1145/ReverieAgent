//! Web capabilities: live multi-engine search and page cleaning.

pub mod engines;
pub mod fetch;

use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::{self, StreamExt};
use reverie_core::domain::ToolOutcome;
use reverie_core::events::SystemEvent;
use serde_json::{json, Value};

use crate::args::{opt_i64_clamped, opt_str, req_str};
use crate::context::ToolExecutionContext;
use crate::error::ToolResult;
use crate::registry::{AgentTool, ToolCategory};
use engines::{SearchHit, SearchRequest, TimeRange};

/// How many result pages may be auto-fetched in one search call.
const MAX_AUTO_FETCH: i64 = 5;

/// Live internet search.
pub struct WebSearchTool;

impl WebSearchTool {
    fn render_hits(query: &str, engine: &str, hits: &[SearchHit]) -> String {
        let mut out = format!("Search results for \"{query}\" via {engine} ({} hits)\n\n", hits.len());
        for (index, hit) in hits.iter().enumerate() {
            out.push_str(&format!("{}. {}\n   URL: {}\n", index + 1, hit.title, hit.url));
            if !hit.snippet.is_empty() {
                out.push_str(&format!("   {}\n", hit.snippet.replace('\n', " ")));
            }
            out.push('\n');
        }
        out
    }
}

#[async_trait]
impl AgentTool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the live internet. Uses configured API backends (Tavily/Brave/Serper/SearXNG) when \
         available and falls back to keyless DuckDuckGo/Bing HTML scraping. Optionally fetches and \
         cleans the full Markdown of the top results in parallel."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Search query"},
                "max_results": {
                    "type": "integer",
                    "description": "Number of organic results to return (1-30)",
                    "minimum": 1,
                    "maximum": 30,
                    "default": 8
                },
                "time_range": {
                    "type": "string",
                    "enum": ["day", "week", "month", "year", "all"],
                    "description": "Recency filter",
                    "default": "all"
                },
                "auto_fetch_top_n": {
                    "type": "integer",
                    "description": "Fetch and clean the full page Markdown of the top N results (0-5)",
                    "minimum": 0,
                    "maximum": 5,
                    "default": 0
                }
            },
            "required": ["query"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Web
    }

    async fn execute(&self, ctx: &ToolExecutionContext, args: Value) -> ToolResult<ToolOutcome> {
        let query = req_str(&args, "query")?;
        if !ctx.web_search_enabled {
            return Ok(ToolOutcome::error(
                "web search is disabled for this session; enable it in the composer or answer from \
                 local context",
            ));
        }

        let max_results = opt_i64_clamped(&args, "max_results", 8, 1, 30) as usize;
        let time_range = TimeRange::parse(&opt_str(&args, "time_range").unwrap_or_else(|| "all".into()));
        let auto_fetch = opt_i64_clamped(&args, "auto_fetch_top_n", 0, 0, MAX_AUTO_FETCH) as usize;

        ctx.emit_activity(format!("Searching web: \"{}\"…", truncate_for_activity(&query)));
        let request = SearchRequest::new(query.clone(), max_results, time_range);
        let (hits, engine) = engines::search(&ctx.http, &ctx.search_config, &request).await?;

        ctx.sink.emit(SystemEvent::SearchProgress {
            session_id: ctx.session_id.clone(),
            agent_id: ctx.agent_id.clone(),
            query: query.clone(),
            engine: engine.clone(),
            results_found: hits.len(),
        });

        let mut output = Self::render_hits(&query, &engine, &hits);

        if auto_fetch > 0 {
            ctx.emit_activity(format!("Reading top {} search results…", auto_fetch.min(hits.len())));
            let targets: Vec<(String, String)> = hits
                .iter()
                .take(auto_fetch)
                .map(|hit| (hit.title.clone(), hit.url.clone()))
                .collect();

            let client = ctx.http.clone();
            let pages: Vec<(String, Result<fetch::FetchedPage, String>)> =
                stream::iter(targets.into_iter().map(|(title, url)| {
                    let client = client.clone();
                    async move {
                        let result = fetch::fetch_clean(&client, &url, None, 12_000)
                            .await
                            .map_err(|err| err.to_string());
                        (title, result)
                    }
                }))
                .buffer_unordered(4)
                .collect()
                .await;

            output.push_str("\n---\n\n# Fetched pages\n\n");
            for (title, result) in pages {
                match result {
                    Ok(page) => {
                        output.push_str(&format!("## {title}\nSource: {}\n\n{}\n\n", page.final_url, page.markdown));
                    }
                    Err(err) => {
                        output.push_str(&format!("## {title}\nFetch failed: {err}\n\n"));
                    }
                }
            }
        }

        let metadata = json!({
            "engine": engine,
            "hits": hits.iter().map(|hit| json!({"title": hit.title, "url": hit.url})).collect::<Vec<_>>(),
        });
        Ok(ToolOutcome::ok_with(ctx.cap_output(output), metadata))
    }
}

fn truncate_for_activity(text: &str) -> String {
    reverie_core::util::truncate_chars(text, 48)
}

/// Fetch one URL and convert it to clean Markdown.
pub struct WebFetchCleanTool;

#[async_trait]
impl AgentTool for WebFetchCleanTool {
    fn name(&self) -> &str {
        "web_fetch_clean"
    }

    fn description(&self) -> &str {
        "Fetch a web page and convert it to clean Markdown, stripping scripts, styles, navigation \
         and footers. Supports CSS selector extraction to isolate the main content."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "Absolute http(s) URL"},
                "selector": {"type": "string", "description": "Optional CSS selector to isolate content"},
                "max_chars": {
                    "type": "integer",
                    "description": "Maximum characters to return",
                    "default": 40000
                }
            },
            "required": ["url"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Web
    }

    async fn execute(&self, ctx: &ToolExecutionContext, args: Value) -> ToolResult<ToolOutcome> {
        let url = req_str(&args, "url")?;
        let selector = opt_str(&args, "selector");
        let max_chars = opt_i64_clamped(&args, "max_chars", 40_000, 1_000, 200_000) as usize;

        ctx.emit_activity(format!("Fetching {}", truncate_for_activity(&url)));
        let page = fetch::fetch_clean(&ctx.http, &url, selector.as_deref(), max_chars).await?;

        let mut output = format!(
            "# {}\nSource: {}\nContent-Type: {}\nBytes: {}\n\n{}",
            if page.title.is_empty() { &page.url } else { &page.title },
            page.final_url,
            page.content_type,
            page.bytes,
            page.markdown
        );
        if page.final_url != url {
            output.push_str(&format!("\n\n(redirected from {url})"));
        }

        Ok(ToolOutcome::ok_with(
            ctx.cap_output(output),
            json!({"url": page.final_url, "title": page.title, "bytes": page.bytes}),
        ))
    }
}

/// Build the two web tools.
pub fn web_tools() -> Vec<Arc<dyn AgentTool>> {
    vec![Arc::new(WebSearchTool), Arc::new(WebFetchCleanTool)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendering_lists_every_hit() {
        let hits = vec![
            SearchHit { title: "A".into(), url: "https://a.com".into(), snippet: "sa".into(), engine: "x".into() },
            SearchHit { title: "B".into(), url: "https://b.com".into(), snippet: String::new(), engine: "x".into() },
        ];
        let text = WebSearchTool::render_hits("q", "duckduckgo", &hits);
        assert!(text.contains("2 hits"));
        assert!(text.contains("1. A"));
        assert!(text.contains("URL: https://b.com"));
    }

    #[test]
    fn schemas_declare_required_fields() {
        let search = WebSearchTool.parameters();
        assert_eq!(search["required"][0], "query");
        assert_eq!(search["properties"]["max_results"]["maximum"], 30);
        let fetch = WebFetchCleanTool.parameters();
        assert_eq!(fetch["required"][0], "url");
    }

    #[test]
    fn tool_names_are_stable() {
        let tools = web_tools();
        assert_eq!(tools[0].name(), "web_search");
        assert_eq!(tools[1].name(), "web_fetch_clean");
    }
}
