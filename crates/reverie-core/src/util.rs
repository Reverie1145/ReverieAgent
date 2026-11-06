//! Small shared utilities: text truncation, slugging, token estimation and
//! MIME classification helpers used by the store and the tool runtime.

use crate::error::{CoreError, CoreResult};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};

/// Return the longest prefix of `s` containing at most `max_chars` characters.
///
/// Slicing a `str` by byte index panics when the index lands inside a
/// multi-byte character, so any "first N characters" operation must go through
/// this helper rather than `&s[..n]`.
pub fn char_prefix(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((index, _)) => &s[..index],
        None => s,
    }
}

/// Truncate a string on a UTF-8 character boundary, appending an ellipsis when
/// the value was actually shortened.
pub fn truncate_chars(input: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut out = String::new();
    for (idx, ch) in input.chars().enumerate() {
        if idx >= max_chars {
            out.push('…');
            return out;
        }
        out.push(ch);
    }
    out
}

/// Keep the head and the tail of a long document, marking the elided middle.
/// Used when composing context for a model with a small window.
pub fn truncate_middle(input: &str, max_chars: usize) -> String {
    let total = input.chars().count();
    if total <= max_chars || max_chars < 64 {
        return truncate_chars(input, max_chars);
    }
    let head_len = max_chars * 2 / 3;
    let tail_len = max_chars - head_len - 8;
    let head: String = input.chars().take(head_len).collect();
    let tail: String = input.chars().skip(total - tail_len).collect();
    format!("{head}\n\n…[{} characters elided]…\n\n{tail}", total - head_len - tail_len)
}

/// Produce a filesystem-safe slug from arbitrary text.
pub fn slugify(input: &str, max_len: usize) -> String {
    let mut out = String::new();
    let mut last_dash = true;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
        if out.len() >= max_len {
            break;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "untitled".to_string()
    } else {
        trimmed
    }
}

/// A deliberately rough token estimate (≈4 characters per token for English,
/// ≈1.6 for CJK). Used only for budgeting decisions, never for billing.
pub fn estimate_tokens(text: &str) -> usize {
    let mut cjk = 0usize;
    let mut other = 0usize;
    for ch in text.chars() {
        let cp = ch as u32;
        let is_cjk = (0x4E00..=0x9FFF).contains(&cp)
            || (0x3040..=0x30FF).contains(&cp)
            || (0xAC00..=0xD7AF).contains(&cp)
            || (0x3400..=0x4DBF).contains(&cp);
        if is_cjk {
            cjk += 1;
        } else {
            other += 1;
        }
    }
    (cjk * 10 + other * 25) / 100 + 1
}

/// Number of bytes a base64 string occupies once decoded.
pub fn base64_decoded_len(encoded: &str) -> usize {
    let trimmed = encoded.trim();
    if trimmed.is_empty() {
        return 0;
    }
    let padding = trimmed.bytes().rev().take_while(|b| *b == b'=').count();
    (trimmed.len() / 4).saturating_mul(3).saturating_sub(padding)
}

/// Encode raw bytes for a data-URL / inline multimodal payload.
pub fn encode_base64(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

/// Decode an inline payload back to bytes.
pub fn decode_base64(encoded: &str) -> CoreResult<Vec<u8>> {
    STANDARD
        .decode(encoded.trim())
        .map_err(|err| CoreError::Validation(format!("invalid base64 payload: {err}")))
}

/// Broad classification of an attachment, driving which extraction path the
/// attachment pipeline takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    Image,
    Pdf,
    Text,
    Data,
    Binary,
}

impl AttachmentKind {
    /// Classify by MIME type, falling back to the file extension.
    pub fn classify(mime_type: &str, file_name: &str) -> Self {
        if mime_type.starts_with("image/") {
            return AttachmentKind::Image;
        }
        if mime_type == "application/pdf" {
            return AttachmentKind::Pdf;
        }
        if mime_type.starts_with("text/")
            || mime_type == "application/json"
            || mime_type == "application/xml"
            || mime_type == "application/x-yaml"
        {
            return AttachmentKind::Data;
        }
        match extension_of(file_name).as_deref() {
            Some(
                "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "go" | "java" | "c" | "h" | "cpp" | "hpp"
                | "cs" | "rb" | "php" | "swift" | "kt" | "scala" | "sh" | "ps1" | "sql" | "toml"
                | "yaml" | "yml" | "ini" | "cfg" | "env" | "gradle" | "lock",
            ) => AttachmentKind::Text,
            Some("md" | "markdown" | "txt" | "log" | "csv" | "tsv" | "json" | "jsonl" | "xml" | "html" | "htm") => {
                AttachmentKind::Data
            }
            Some("png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "svg") => AttachmentKind::Image,
            Some("pdf") => AttachmentKind::Pdf,
            _ => AttachmentKind::Binary,
        }
    }

    /// Whether the attachment can be rendered directly into a vision request.
    pub fn is_vision_capable(self) -> bool {
        matches!(self, AttachmentKind::Image)
    }
}

/// Lowercase file extension without the dot.
pub fn extension_of(file_name: &str) -> Option<String> {
    let path = std::path::Path::new(file_name);
    path.extension().map(|ext| ext.to_string_lossy().to_ascii_lowercase())
}

/// Best-effort MIME resolution from a file name.
pub fn guess_mime(file_name: &str) -> String {
    mime_guess::from_path(file_name)
        .first_raw()
        .map(|m| m.to_string())
        .unwrap_or_else(|| "application/octet-stream".to_string())
}

/// Render a byte count for the UI (`1.4 MB`, `812 B`, …).
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Extract fenced code blocks from a Markdown document. Used when a sub-agent
/// returns a patch instead of calling the file tools directly.
pub fn extract_fenced_blocks(markdown: &str) -> Vec<(Option<String>, String)> {
    let mut blocks = Vec::new();
    let mut in_block = false;
    let mut lang: Option<String> = None;
    let mut buffer = String::new();
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            if in_block {
                blocks.push((lang.take(), std::mem::take(&mut buffer)));
                in_block = false;
            } else {
                in_block = true;
                let tag = trimmed.trim_start_matches('`').trim();
                lang = if tag.is_empty() { None } else { Some(tag.to_string()) };
            }
            continue;
        }
        if in_block {
            buffer.push_str(line);
            buffer.push('\n');
        }
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_respects_char_boundaries() {
        let text = "日本語のテキストです";
        let cut = truncate_chars(text, 3);
        assert!(cut.starts_with("日本語"));
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn middle_truncation_keeps_both_ends() {
        let text: String = "abcdefghij".repeat(100);
        let out = truncate_middle(&text, 120);
        assert!(out.starts_with("abcdefghij"));
        assert!(out.contains("elided"));
    }

    #[test]
    fn slugify_strips_punctuation() {
        assert_eq!(slugify("Hello, World! 2024", 40), "hello-world-2024");
        assert_eq!(slugify("***", 10), "untitled");
    }

    #[test]
    fn classification_prefers_mime_then_extension() {
        assert_eq!(AttachmentKind::classify("image/png", "x.bin"), AttachmentKind::Image);
        assert_eq!(AttachmentKind::classify("application/octet-stream", "main.rs"), AttachmentKind::Text);
        assert_eq!(AttachmentKind::classify("application/pdf", "doc.pdf"), AttachmentKind::Pdf);
        assert_eq!(AttachmentKind::classify("text/csv", "data.bin"), AttachmentKind::Data);
    }

    #[test]
    fn base64_length_roundtrip() {
        let payload = b"reverie agent";
        let encoded = encode_base64(payload);
        assert_eq!(base64_decoded_len(&encoded), payload.len());
        assert_eq!(decode_base64(&encoded).unwrap(), payload.to_vec());
    }

    #[test]
    fn fenced_blocks_are_extracted_with_language() {
        let md = "intro\n```rust\nfn main() {}\n```\ntail";
        let blocks = extract_fenced_blocks(md);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].0.as_deref(), Some("rust"));
        assert!(blocks[0].1.contains("fn main"));
    }

    #[test]
    fn human_bytes_formats_units() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KB");
    }
}
