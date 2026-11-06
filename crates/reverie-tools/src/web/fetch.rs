//! High-fidelity web fetching and HTML → Markdown cleaning.

use std::sync::OnceLock;

use regex::Regex;
use scraper::{Html, Selector};

use crate::error::{ToolError, ToolResult};

/// A fetched and cleaned page.
#[derive(Debug, Clone)]
pub struct FetchedPage {
    pub url: String,
    pub final_url: String,
    pub status: u16,
    pub content_type: String,
    pub title: String,
    pub markdown: String,
    pub bytes: usize,
}

const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                          (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Tags whose entire subtree carries no readable content.
const NOISE_TAGS: [&str; 9] = ["script", "style", "noscript", "svg", "iframe", "nav", "footer", "aside", "form"];

fn noise_patterns() -> &'static Vec<Regex> {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let mut patterns = Vec::new();
        for tag in NOISE_TAGS {
            let source = format!(r"(?is)<{tag}\b[^>]*>.*?</{tag}\s*>");
            if let Ok(regex) = Regex::new(&source) {
                patterns.push(regex);
            }
        }
        if let Ok(comments) = Regex::new(r"(?is)<!--.*?-->") {
            patterns.push(comments);
        }
        patterns
    })
}

/// Remove script/style/nav/footer subtrees and HTML comments.
pub fn strip_noise(html: &str) -> String {
    let mut out = html.to_string();
    for pattern in noise_patterns() {
        if pattern.is_match(&out) {
            out = pattern.replace_all(&out, " ").into_owned();
        }
    }
    out
}

/// Extract the document title, preferring `<title>` and falling back to `<h1>`.
pub fn extract_title(document: &Html) -> String {
    if let Ok(selector) = Selector::parse("title") {
        if let Some(node) = document.select(&selector).next() {
            let title = node.text().collect::<Vec<_>>().join(" ");
            let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
            if !title.is_empty() {
                return title;
            }
        }
    }
    if let Ok(selector) = Selector::parse("h1") {
        if let Some(node) = document.select(&selector).next() {
            let heading = node.text().collect::<Vec<_>>().join(" ");
            let heading = heading.split_whitespace().collect::<Vec<_>>().join(" ");
            if !heading.is_empty() {
                return heading;
            }
        }
    }
    String::new()
}

/// Convert HTML to Markdown, collapsing runs of blank lines.
pub fn html_to_markdown(html: &str) -> String {
    let cleaned = strip_noise(html);
    let markdown = htmd::convert(&cleaned).unwrap_or_else(|_| cleaned.clone());
    let mut out = String::with_capacity(markdown.len());
    let mut blank_run = 0usize;
    for line in markdown.lines() {
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
            out.push('\n');
        } else {
            blank_run = 0;
            out.push_str(line.trim_end());
            out.push('\n');
        }
    }
    out.trim().to_string()
}

/// Keep only the subtrees matching a CSS selector, then convert them.
pub fn extract_by_selector(html: &str, selector: &str) -> ToolResult<String> {
    let document = Html::parse_document(html);
    let parsed = Selector::parse(selector)
        .map_err(|err| ToolError::invalid(format!("invalid CSS selector `{selector}`: {err}")))?;
    let mut parts: Vec<String> = Vec::new();
    for node in document.select(&parsed) {
        let outer = node.html();
        let markdown = html_to_markdown(&outer);
        if !markdown.trim().is_empty() {
            parts.push(markdown);
        }
    }
    if parts.is_empty() {
        return Err(ToolError::NotFound(format!("selector `{selector}` matched no content")));
    }
    Ok(parts.join("\n\n"))
}

/// Fetch a URL and clean it into Markdown.
pub async fn fetch_clean(
    client: &reqwest::Client,
    url: &str,
    selector: Option<&str>,
    max_chars: usize,
) -> ToolResult<FetchedPage> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(ToolError::invalid("url must start with http:// or https://"));
    }

    let response = client
        .get(url)
        .header("User-Agent", BROWSER_UA)
        .header("Accept", "text/html,application/xhtml+xml,application/json,text/plain;q=0.9,*/*;q=0.8")
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await?;

    let status = response.status().as_u16();
    let final_url = response.url().to_string();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("text/html")
        .to_string();

    if status >= 400 {
        return Err(ToolError::other(format!("{url} returned HTTP {status}")));
    }

    let body = response.text().await?;
    let bytes = body.len();

    let looks_html = content_type.contains("html") || body.trim_start().starts_with('<');

    if !looks_html {
        let truncated = reverie_core::util::truncate_middle(&body, max_chars.max(1000));
        let title = if content_type.contains("json") { "JSON document" } else { "Plain text" };
        return Ok(FetchedPage {
            url: url.to_string(),
            final_url,
            status,
            content_type,
            title: title.to_string(),
            markdown: truncated,
            bytes,
        });
    }

    let document = Html::parse_document(&body);
    let title = extract_title(&document);

    let markdown = match selector {
        Some(selector) => extract_by_selector(&body, selector)?,
        None => html_to_markdown(&body),
    };

    if markdown.trim().is_empty() {
        return Err(ToolError::NotFound(format!("{url} produced no readable content")));
    }

    let markdown = reverie_core::util::truncate_middle(&markdown, max_chars.max(1000));

    Ok(FetchedPage { url: url.to_string(), final_url, status, content_type, title, markdown, bytes })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
        <html>
          <head><title>Rust Async Guide</title><style>body{color:red}</style></head>
          <body>
            <nav><a href="/">Home</a></nav>
            <script>console.log("noise")</script>
            <h1>Async Rust</h1>
            <p>Tokio is an async runtime.</p>
            <ul><li>Futures</li><li>Actors</li></ul>
            <footer>Copyright</footer>
          </body>
        </html>
    "#;

    #[test]
    fn noise_is_stripped() {
        let cleaned = strip_noise(SAMPLE);
        assert!(!cleaned.contains("console.log"));
        assert!(!cleaned.contains("color:red"));
        assert!(!cleaned.contains("Copyright"));
        assert!(cleaned.contains("Async Rust"));
    }

    #[test]
    fn markdown_conversion_keeps_structure() {
        let markdown = html_to_markdown(SAMPLE);
        assert!(markdown.contains("Async Rust"));
        assert!(markdown.contains("Tokio is an async runtime."));
        assert!(markdown.contains("Futures"));
        assert!(!markdown.contains("Copyright"));
    }

    #[test]
    fn title_prefers_title_tag() {
        let document = Html::parse_document(SAMPLE);
        assert_eq!(extract_title(&document), "Rust Async Guide");
    }

    #[test]
    fn title_falls_back_to_h1() {
        let document = Html::parse_document("<html><body><h1>Heading Only</h1></body></html>");
        assert_eq!(extract_title(&document), "Heading Only");
    }

    #[test]
    fn selector_extraction_works() {
        let html = "<div class='a'><p>First</p></div><div class='a'><p>Second</p></div>";
        let out = extract_by_selector(html, ".a").unwrap();
        assert!(out.contains("First"));
        assert!(out.contains("Second"));
    }

    #[test]
    fn bad_selector_is_reported() {
        assert!(extract_by_selector("<p>x</p>", ":::").is_err());
    }

    #[test]
    fn missing_selector_match_errors() {
        assert!(extract_by_selector("<p>x</p>", ".missing").is_err());
    }

    #[test]
    fn blank_line_runs_collapse() {
        let markdown = html_to_markdown("<p>a</p><p></p><p></p><p>b</p>");
        assert!(!markdown.contains("\n\n\n"));
    }
}
