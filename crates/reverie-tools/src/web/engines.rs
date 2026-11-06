//! Multi-engine web search.
//!
//! Order of preference: an explicitly preferred engine, then any API-key
//! backend (Tavily → Brave → Serper → SearXNG), then the keyless HTML scrapers
//! (DuckDuckGo → Bing). Every backend normalizes into [`SearchHit`].

use reverie_core::domain::SearchEngineConfig;
use scraper::{Html, Selector};
use serde_json::json;
use urlencoding::decode;

use crate::error::{ToolError, ToolResult};

/// A single organic result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub engine: String,
}

/// Recency filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeRange {
    Day,
    Week,
    Month,
    Year,
    All,
}

impl TimeRange {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "day" | "d" | "24h" => TimeRange::Day,
            "week" | "w" => TimeRange::Week,
            "month" | "m" => TimeRange::Month,
            "year" | "y" => TimeRange::Year,
            _ => TimeRange::All,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TimeRange::Day => "day",
            TimeRange::Week => "week",
            TimeRange::Month => "month",
            TimeRange::Year => "year",
            TimeRange::All => "all",
        }
    }

    /// DuckDuckGo `df` parameter.
    fn duckduckgo_param(self) -> Option<&'static str> {
        match self {
            TimeRange::Day => Some("d"),
            TimeRange::Week => Some("w"),
            TimeRange::Month => Some("m"),
            TimeRange::Year => Some("y"),
            TimeRange::All => None,
        }
    }

    /// Brave `freshness` parameter.
    fn brave_param(self) -> Option<&'static str> {
        match self {
            TimeRange::Day => Some("pd"),
            TimeRange::Week => Some("pw"),
            TimeRange::Month => Some("pm"),
            TimeRange::Year => Some("py"),
            TimeRange::All => None,
        }
    }

    /// Tavily `days` parameter.
    fn tavily_days(self) -> Option<i64> {
        match self {
            TimeRange::Day => Some(1),
            TimeRange::Week => Some(7),
            TimeRange::Month => Some(31),
            TimeRange::Year => Some(365),
            TimeRange::All => None,
        }
    }

    /// Serper `tbs` parameter.
    fn serper_param(self) -> Option<&'static str> {
        match self {
            TimeRange::Day => Some("qdr:d"),
            TimeRange::Week => Some("qdr:w"),
            TimeRange::Month => Some("qdr:m"),
            TimeRange::Year => Some("qdr:y"),
            TimeRange::All => None,
        }
    }
}

/// A normalized search request.
#[derive(Debug, Clone)]
pub struct SearchRequest {
    pub query: String,
    pub max_results: usize,
    pub time_range: TimeRange,
}

impl SearchRequest {
    pub fn new(query: impl Into<String>, max_results: usize, time_range: TimeRange) -> Self {
        SearchRequest { query: query.into(), max_results: max_results.clamp(1, 30), time_range }
    }
}

const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                          (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

fn collapse_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn element_text(element: &scraper::ElementRef<'_>) -> String {
    collapse_ws(&element.text().collect::<Vec<_>>().join(" "))
}

/// Unwrap DuckDuckGo's `//duckduckgo.com/l/?uddg=<encoded>` redirect wrapper.
pub fn clean_duckduckgo_url(raw: &str) -> String {
    let candidate = if raw.starts_with("//") { format!("https:{raw}") } else { raw.to_string() };
    if let Some(index) = candidate.find("uddg=") {
        let tail = &candidate[index + "uddg=".len()..];
        let encoded = tail.split('&').next().unwrap_or(tail);
        if let Ok(decoded) = decode(encoded) {
            return decoded.into_owned();
        }
    }
    candidate
}

fn is_http(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Parse a DuckDuckGo HTML endpoint response.
pub fn parse_duckduckgo_html(body: &str, limit: usize) -> Vec<SearchHit> {
    let document = Html::parse_document(body);
    let row = Selector::parse(".result, .web-result").expect("static selector");
    let link = Selector::parse("a.result__a").expect("static selector");
    let snippet = Selector::parse(".result__snippet").expect("static selector");

    let mut hits = Vec::new();
    for node in document.select(&row) {
        let Some(anchor) = node.select(&link).next() else {
            continue;
        };
        let title = element_text(&anchor);
        let href = anchor.value().attr("href").unwrap_or_default();
        let url = clean_duckduckgo_url(href);
        if title.is_empty() || !is_http(&url) {
            continue;
        }
        let snippet = node.select(&snippet).next().map(|node| element_text(&node)).unwrap_or_default();
        hits.push(SearchHit { title, url, snippet, engine: "duckduckgo".to_string() });
        if hits.len() >= limit {
            break;
        }
    }
    hits
}

/// Parse the DuckDuckGo Lite endpoint response.
pub fn parse_duckduckgo_lite(body: &str, limit: usize) -> Vec<SearchHit> {
    let document = Html::parse_document(body);
    let link = Selector::parse("a.result-link").expect("static selector");
    let snippet = Selector::parse(".result-snippet").expect("static selector");

    let mut hits: Vec<SearchHit> = Vec::new();
    for anchor in document.select(&link) {
        let title = element_text(&anchor);
        let href = anchor.value().attr("href").unwrap_or_default();
        let url = clean_duckduckgo_url(href);
        if title.is_empty() || !is_http(&url) {
            continue;
        }
        hits.push(SearchHit { title, url, snippet: String::new(), engine: "duckduckgo".to_string() });
        if hits.len() >= limit {
            break;
        }
    }
    // Attach snippets positionally; the lite layout emits one snippet per row.
    let snippets: Vec<String> = document.select(&snippet).map(|node| element_text(&node)).collect();
    for (index, hit) in hits.iter_mut().enumerate() {
        if let Some(value) = snippets.get(index) {
            hit.snippet = value.clone();
        }
    }
    hits
}

/// Parse a Bing web results page.
pub fn parse_bing(body: &str, limit: usize) -> Vec<SearchHit> {
    let document = Html::parse_document(body);
    let row = Selector::parse("li.b_algo").expect("static selector");
    let link = Selector::parse("h2 a").expect("static selector");
    let snippet = Selector::parse(".b_caption p, .b_algoSlug, p").expect("static selector");

    let mut hits = Vec::new();
    for node in document.select(&row) {
        let Some(anchor) = node.select(&link).next() else {
            continue;
        };
        let title = element_text(&anchor);
        let url = anchor.value().attr("href").unwrap_or_default().to_string();
        if title.is_empty() || !is_http(&url) {
            continue;
        }
        let snippet_text = node.select(&snippet).next().map(|node| element_text(&node)).unwrap_or_default();
        hits.push(SearchHit { title, url, snippet: snippet_text, engine: "bing".to_string() });
        if hits.len() >= limit {
            break;
        }
    }
    hits
}

async fn duckduckgo(client: &reqwest::Client, request: &SearchRequest) -> ToolResult<Vec<SearchHit>> {
    let mut form = vec![("q".to_string(), request.query.clone()), ("kl".to_string(), "wt-wt".to_string())];
    if let Some(df) = request.time_range.duckduckgo_param() {
        form.push(("df".to_string(), df.to_string()));
    }

    let response = client
        .post("https://html.duckduckgo.com/html/")
        .header("User-Agent", BROWSER_UA)
        .header("Accept-Language", "en-US,en;q=0.9")
        .form(&form)
        .send()
        .await?;
    if response.status().is_success() {
        let body = response.text().await?;
        let hits = parse_duckduckgo_html(&body, request.max_results);
        if !hits.is_empty() {
            return Ok(hits);
        }
    }

    // Fall back to the lite endpoint, which survives some regional blocks.
    let url = format!("https://lite.duckduckgo.com/lite/?q={}", urlencoding::encode(&request.query));
    let response = client
        .get(&url)
        .header("User-Agent", BROWSER_UA)
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(ToolError::other(format!("DuckDuckGo returned {}", response.status())));
    }
    let body = response.text().await?;
    Ok(parse_duckduckgo_lite(&body, request.max_results))
}

async fn bing(client: &reqwest::Client, request: &SearchRequest) -> ToolResult<Vec<SearchHit>> {
    let url = format!(
        "https://www.bing.com/search?q={}&count={}&setlang=en",
        urlencoding::encode(&request.query),
        request.max_results
    );
    let response = client
        .get(&url)
        .header("User-Agent", BROWSER_UA)
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(ToolError::other(format!("Bing returned {}", response.status())));
    }
    let body = response.text().await?;
    Ok(parse_bing(&body, request.max_results))
}

async fn tavily(
    client: &reqwest::Client,
    api_key: &str,
    request: &SearchRequest,
) -> ToolResult<Vec<SearchHit>> {
    let mut body = json!({
        "api_key": api_key,
        "query": request.query,
        "max_results": request.max_results,
        "search_depth": "basic",
        "include_answer": false,
    });
    if let Some(days) = request.time_range.tavily_days() {
        body["days"] = json!(days);
    }
    let response = client.post("https://api.tavily.com/search").json(&body).send().await?;
    if !response.status().is_success() {
        return Err(ToolError::other(format!("Tavily returned {}", response.status())));
    }
    let payload: serde_json::Value = response.json().await?;
    Ok(payload
        .get("results")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let url = item.get("url").and_then(|v| v.as_str())?;
                    Some(SearchHit {
                        title: item.get("title").and_then(|v| v.as_str()).unwrap_or(url).to_string(),
                        url: url.to_string(),
                        snippet: item.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        engine: "tavily".to_string(),
                    })
                })
                .take(request.max_results)
                .collect()
        })
        .unwrap_or_default())
}

async fn brave(client: &reqwest::Client, api_key: &str, request: &SearchRequest) -> ToolResult<Vec<SearchHit>> {
    let mut url = format!(
        "https://api.search.brave.com/res/v1/web/search?q={}&count={}",
        urlencoding::encode(&request.query),
        request.max_results
    );
    if let Some(freshness) = request.time_range.brave_param() {
        url.push_str(&format!("&freshness={freshness}"));
    }
    let response = client
        .get(&url)
        .header("X-Subscription-Token", api_key)
        .header("Accept", "application/json")
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(ToolError::other(format!("Brave returned {}", response.status())));
    }
    let payload: serde_json::Value = response.json().await?;
    Ok(payload["web"]["results"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let url = item.get("url").and_then(|v| v.as_str())?;
                    Some(SearchHit {
                        title: item.get("title").and_then(|v| v.as_str()).unwrap_or(url).to_string(),
                        url: url.to_string(),
                        snippet: item.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        engine: "brave".to_string(),
                    })
                })
                .take(request.max_results)
                .collect()
        })
        .unwrap_or_default())
}

async fn serper(client: &reqwest::Client, api_key: &str, request: &SearchRequest) -> ToolResult<Vec<SearchHit>> {
    let mut body = json!({"q": request.query, "num": request.max_results});
    if let Some(tbs) = request.time_range.serper_param() {
        body["tbs"] = json!(tbs);
    }
    let response = client
        .post("https://google.serper.dev/search")
        .header("X-API-KEY", api_key)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(ToolError::other(format!("Serper returned {}", response.status())));
    }
    let payload: serde_json::Value = response.json().await?;
    Ok(payload["organic"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let url = item.get("link").and_then(|v| v.as_str())?;
                    Some(SearchHit {
                        title: item.get("title").and_then(|v| v.as_str()).unwrap_or(url).to_string(),
                        url: url.to_string(),
                        snippet: item.get("snippet").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        engine: "serper".to_string(),
                    })
                })
                .take(request.max_results)
                .collect()
        })
        .unwrap_or_default())
}

async fn searxng(
    client: &reqwest::Client,
    endpoint: &str,
    request: &SearchRequest,
) -> ToolResult<Vec<SearchHit>> {
    let base = endpoint.trim_end_matches('/');
    let url = format!(
        "{base}/search?q={}&format=json&safesearch=0",
        urlencoding::encode(&request.query)
    );
    let response = client.get(&url).header("User-Agent", BROWSER_UA).send().await?;
    if !response.status().is_success() {
        return Err(ToolError::other(format!("SearXNG returned {}", response.status())));
    }
    let payload: serde_json::Value = response.json().await?;
    Ok(payload["results"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let url = item.get("url").and_then(|v| v.as_str())?;
                    Some(SearchHit {
                        title: item.get("title").and_then(|v| v.as_str()).unwrap_or(url).to_string(),
                        url: url.to_string(),
                        snippet: item.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        engine: "searxng".to_string(),
                    })
                })
                .take(request.max_results)
                .collect()
        })
        .unwrap_or_default())
}

fn non_empty(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|value| !value.is_empty())
}

/// Run a search, trying backends in priority order until one yields results.
///
/// Returns the hits plus the name of the engine that produced them.
pub async fn search(
    client: &reqwest::Client,
    config: &SearchEngineConfig,
    request: &SearchRequest,
) -> ToolResult<(Vec<SearchHit>, String)> {
    if request.query.trim().is_empty() {
        return Err(ToolError::invalid("query must not be empty"));
    }

    let mut attempts: Vec<(&'static str, ToolResult<Vec<SearchHit>>)> = Vec::new();
    let preferred = config.preferred_engine.as_deref().unwrap_or("auto").to_ascii_lowercase();

    let mut backends: Vec<&str> = Vec::new();
    if preferred != "auto" {
        backends.push(match preferred.as_str() {
            "tavily" => "tavily",
            "brave" => "brave",
            "serper" => "serper",
            "searxng" => "searxng",
            "bing" => "bing",
            _ => "duckduckgo",
        });
    }
    for candidate in ["tavily", "brave", "serper", "searxng", "duckduckgo", "bing"] {
        if !backends.contains(&candidate) {
            backends.push(candidate);
        }
    }

    for backend in backends {
        let result = match backend {
            "tavily" => match non_empty(&config.tavily_api_key) {
                Some(key) => tavily(client, key, request).await,
                None => continue,
            },
            "brave" => match non_empty(&config.brave_api_key) {
                Some(key) => brave(client, key, request).await,
                None => continue,
            },
            "serper" => match non_empty(&config.serper_api_key) {
                Some(key) => serper(client, key, request).await,
                None => continue,
            },
            "searxng" => match non_empty(&config.searxng_endpoint) {
                Some(endpoint) => searxng(client, endpoint, request).await,
                None => continue,
            },
            "bing" => bing(client, request).await,
            _ => duckduckgo(client, request).await,
        };

        match result {
            Ok(hits) if !hits.is_empty() => return Ok((hits, backend.to_string())),
            Ok(_) => attempts.push((backend, Ok(Vec::new()))),
            Err(err) => attempts.push((backend, Err(err))),
        }
    }

    let failures: Vec<String> = attempts
        .iter()
        .map(|(name, outcome)| match outcome {
            Ok(_) => format!("{name}: no results"),
            Err(err) => format!("{name}: {err}"),
        })
        .collect();
    Err(ToolError::other(format!(
        "every search backend failed ({})",
        failures.join("; ")
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duckduckgo_redirect_is_unwrapped() {
        let raw = "//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.rust-lang.org%2F&rut=abc";
        assert_eq!(clean_duckduckgo_url(raw), "https://www.rust-lang.org/");
        assert_eq!(clean_duckduckgo_url("https://example.com"), "https://example.com");
    }

    #[test]
    fn ddg_html_parsing_extracts_hits() {
        let body = r#"
            <div class="result">
              <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Ftokio.rs%2F">Tokio</a>
              <a class="result__snippet">Async runtime for Rust</a>
            </div>
            <div class="result">
              <a class="result__a" href="https://example.com/x">Example</a>
              <a class="result__snippet">Another result</a>
            </div>
        "#;
        let hits = parse_duckduckgo_html(body, 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].url, "https://tokio.rs/");
        assert_eq!(hits[0].snippet, "Async runtime for Rust");
        assert_eq!(hits[1].engine, "duckduckgo");
    }

    #[test]
    fn ddg_parsing_respects_limit() {
        let body = r#"
            <div class="result"><a class="result__a" href="https://a.com">A</a></div>
            <div class="result"><a class="result__a" href="https://b.com">B</a></div>
            <div class="result"><a class="result__a" href="https://c.com">C</a></div>
        "#;
        assert_eq!(parse_duckduckgo_html(body, 2).len(), 2);
    }

    #[test]
    fn bing_parsing_extracts_hits() {
        let body = r#"
            <li class="b_algo">
              <h2><a href="https://docs.rs/sqlx">sqlx</a></h2>
              <div class="b_caption"><p>The Rust SQL toolkit</p></div>
            </li>
        "#;
        let hits = parse_bing(body, 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://docs.rs/sqlx");
        assert_eq!(hits[0].snippet, "The Rust SQL toolkit");
        assert_eq!(hits[0].engine, "bing");
    }

    #[test]
    fn non_http_links_are_dropped() {
        let body = r#"<div class="result"><a class="result__a" href="/relative">Rel</a></div>"#;
        assert!(parse_duckduckgo_html(body, 5).is_empty());
    }

    #[test]
    fn time_range_parsing() {
        assert_eq!(TimeRange::parse("DAY"), TimeRange::Day);
        assert_eq!(TimeRange::parse("nonsense"), TimeRange::All);
        assert_eq!(TimeRange::Week.as_str(), "week");
        assert_eq!(TimeRange::Day.duckduckgo_param(), Some("d"));
        assert_eq!(TimeRange::Month.tavily_days(), Some(31));
        assert_eq!(TimeRange::All.serper_param(), None);
    }

    #[test]
    fn request_clamps_result_count() {
        assert_eq!(SearchRequest::new("q", 99, TimeRange::All).max_results, 30);
        assert_eq!(SearchRequest::new("q", 0, TimeRange::All).max_results, 1);
    }
}
