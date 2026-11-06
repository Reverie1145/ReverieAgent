//! SSE plumbing shared by the streaming adapters.

use eventsource_stream::Eventsource;
use futures::stream::BoxStream;
use futures::{Stream, StreamExt};
use reverie_core::llm::StreamChunk;

use crate::error::{LlmError, LlmResult};

/// One server-sent event.
pub type SseEvent = eventsource_stream::Event;

/// Opening tag used by local reasoning models that inline their thinking.
pub const THINK_OPEN: &str = "<think>";
/// Closing tag paired with [`THINK_OPEN`].
pub const THINK_CLOSE: &str = "</think>";

/// Adapt a `reqwest` byte stream into a stream of SSE events.
pub fn sse_stream(response: reqwest::Response) -> impl Stream<Item = LlmResult<SseEvent>> + Send + 'static {
    response
        .bytes_stream()
        .map(|item| item.map_err(LlmError::Http))
        .eventsource()
        .map(|item| item.map_err(|err| LlmError::Protocol(format!("malformed SSE frame: {err}"))))
}

/// Adapt a `reqwest` byte stream into newline-delimited JSON objects, the
/// framing used by Ollama.
pub fn ndjson_stream(response: reqwest::Response) -> BoxStream<'static, LlmResult<serde_json::Value>> {
    use bytes::BytesMut;
    futures::stream::unfold(
        (response.bytes_stream().boxed(), BytesMut::new(), false),
        |(mut stream, mut buffer, done)| async move {
            if done {
                return None;
            }
            loop {
                if let Some(pos) = buffer.iter().position(|byte| *byte == b'\n') {
                    let line: Vec<u8> = buffer.split_to(pos + 1).to_vec();
                    let text = String::from_utf8_lossy(&line).trim().to_string();
                    if text.is_empty() {
                        continue;
                    }
                    return match serde_json::from_str::<serde_json::Value>(&text) {
                        Ok(value) => Some((Ok(value), (stream, buffer, false))),
                        Err(err) => Some((
                            Err(LlmError::Protocol(format!("invalid NDJSON line: {err}"))),
                            (stream, buffer, false),
                        )),
                    };
                }
                match stream.next().await {
                    Some(Ok(chunk)) => buffer.extend_from_slice(&chunk),
                    Some(Err(err)) => return Some((Err(LlmError::Http(err)), (stream, buffer, true))),
                    None => {
                        let text = String::from_utf8_lossy(&buffer).trim().to_string();
                        buffer.clear();
                        if text.is_empty() {
                            return None;
                        }
                        return match serde_json::from_str::<serde_json::Value>(&text) {
                            Ok(value) => Some((Ok(value), (stream, buffer, true))),
                            Err(err) => Some((
                                Err(LlmError::Protocol(format!("invalid trailing NDJSON: {err}"))),
                                (stream, buffer, true),
                            )),
                        };
                    }
                }
            }
        },
    )
    .boxed()
}

/// Incrementally separates inline `&lt;think&gt;…&lt;/think&gt;` reasoning from visible
/// text. Local models often emit reasoning inline rather than in a dedicated
/// field; this keeps the UI's thinking pane accurate.
#[derive(Debug, Default)]
pub struct ThinkTagSplitter {
    inside: bool,
    pending: String,
}

/// Result of feeding a delta through the splitter.
#[derive(Debug, Default, PartialEq)]
pub struct SplitDelta {
    pub visible: String,
    pub reasoning: String,
}

impl ThinkTagSplitter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_inside(&self) -> bool {
        self.inside
    }

    /// Feed a text delta and receive the routed halves.
    pub fn push(&mut self, delta: &str) -> SplitDelta {
        let mut out = SplitDelta::default();
        let mut work = std::mem::take(&mut self.pending);
        work.push_str(delta);

        loop {
            let open = work.find(THINK_OPEN);
            let close = work.find(THINK_CLOSE);

            match (self.inside, open, close) {
                (false, Some(idx), _) => {
                    out.visible.push_str(&work[..idx]);
                    work = work[idx + THINK_OPEN.len()..].to_string();
                    self.inside = true;
                }
                (true, _, Some(idx)) => {
                    out.reasoning.push_str(&work[..idx]);
                    work = work[idx + THINK_CLOSE.len()..].to_string();
                    self.inside = false;
                }
                _ => break,
            }
        }

        let hold = partial_tag_suffix(&work);
        if hold > 0 {
            self.pending = work[work.len() - hold..].to_string();
            work.truncate(work.len() - hold);
        }

        if self.inside {
            out.reasoning.push_str(&work);
        } else {
            out.visible.push_str(&work);
        }
        out
    }

    /// Flush any buffered partial tag at end of stream.
    pub fn finish(&mut self) -> SplitDelta {
        let mut out = SplitDelta::default();
        let rest = std::mem::take(&mut self.pending);
        if rest.is_empty() {
            return out;
        }
        if self.inside {
            out.reasoning.push_str(&rest);
        } else {
            out.visible.push_str(&rest);
        }
        out
    }
}

/// Length of the longest suffix of `text` that is a proper prefix of one of the
/// think tags, so a tag split across deltas is never emitted prematurely.
fn partial_tag_suffix(text: &str) -> usize {
    const TAGS: [&str; 2] = [THINK_OPEN, THINK_CLOSE];
    let bytes = text.as_bytes();
    let mut best = 0usize;
    for tag in TAGS {
        let tag_bytes = tag.as_bytes();
        let max = tag_bytes.len().min(bytes.len() + 1);
        for len in (1..max).rev() {
            if bytes.len() >= len && bytes.ends_with(&tag_bytes[..len]) {
                best = best.max(len);
                break;
            }
        }
    }
    best
}

/// Accumulates streamed tool-call fragments into complete calls.
#[derive(Debug, Default)]
pub struct ToolCallAccumulator {
    slots: Vec<ToolCallSlot>,
}

#[derive(Debug, Default, Clone)]
struct ToolCallSlot {
    id: String,
    name: String,
    arguments: String,
}

impl ToolCallAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, index: usize, id: Option<&str>, name: Option<&str>, args_delta: &str) {
        while self.slots.len() <= index {
            self.slots.push(ToolCallSlot::default());
        }
        let slot = &mut self.slots[index];
        if let Some(id) = id {
            if !id.is_empty() {
                slot.id = id.to_string();
            }
        }
        if let Some(name) = name {
            if !name.is_empty() {
                slot.name.push_str(name);
            }
        }
        slot.arguments.push_str(args_delta);
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Consume the accumulated fragments into finalized tool calls.
    pub fn drain(&mut self) -> Vec<reverie_core::llm::ToolCall> {
        let slots = std::mem::take(&mut self.slots);
        slots
            .into_iter()
            .filter(|slot| !slot.name.trim().is_empty())
            .enumerate()
            .map(|(idx, slot)| reverie_core::llm::ToolCall {
                id: if slot.id.is_empty() { format!("call_{idx}") } else { slot.id },
                name: slot.name.trim().to_string(),
                arguments: reverie_core::llm::ToolCall::parse_arguments(&slot.arguments),
            })
            .collect()
    }
}

/// Wrap a single text delta into the canonical chunk shape.
pub fn text_chunk(text: impl Into<String>) -> StreamChunk {
    StreamChunk::TextDelta(text.into())
}

/// Create a bounded channel plus the matching output stream.
///
/// Adapters decode the provider stream inside a spawned task and forward chunks
/// through this channel, which keeps the returned `BoxStream` free of borrows
/// on the HTTP response and therefore `'static + Send`.
pub fn channel_pair() -> (tokio::sync::mpsc::Sender<LlmResult<StreamChunk>>, BoxStream<'static, LlmResult<StreamChunk>>) {
    let (tx, rx) = tokio::sync::mpsc::channel::<LlmResult<StreamChunk>>(512);
    let stream = futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|item| (item, rx)) });
    (tx, stream.boxed())
}

/// Send a chunk, ignoring the error produced when the consumer hung up.
pub async fn forward(tx: &tokio::sync::mpsc::Sender<LlmResult<StreamChunk>>, chunk: StreamChunk) -> bool {
    tx.send(Ok(chunk)).await.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitter_routes_inline_reasoning() {
        let mut splitter = ThinkTagSplitter::new();
        let first = splitter.push(&format!("hello {}planning", "<think>"));
        assert_eq!(first.visible, "hello ");
        assert_eq!(first.reasoning, "planning");
        let second = splitter.push(&format!(" more{}{}answer", "<", "/think>"));
        assert_eq!(second.reasoning, " more");
        assert_eq!(second.visible, "answer");
    }

    #[test]
    fn splitter_handles_tag_split_across_deltas() {
        let mut splitter = ThinkTagSplitter::new();
        let a = splitter.push("abc <thi");
        assert_eq!(a.visible, "abc ");
        let b = splitter.push("nk>secret</thi");
        assert_eq!(b.reasoning, "secret");
        let c = splitter.push("nk>done");
        assert_eq!(c.visible, "done");
    }

    #[test]
    fn splitter_flush_returns_held_text() {
        let mut splitter = ThinkTagSplitter::new();
        let out = splitter.push("tail <thi");
        assert_eq!(out.visible, "tail ");
        let flushed = splitter.finish();
        assert_eq!(flushed.visible, "<thi");
    }

    #[test]
    fn accumulator_merges_fragments() {
        let mut acc = ToolCallAccumulator::new();
        acc.push(0, Some("call_1"), Some("web_"), "");
        acc.push(0, None, Some("search"), "{\"query\":");
        acc.push(0, None, None, "\"rust\"}");
        let calls = acc.drain();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web_search");
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].arguments["query"], "rust");
        assert!(acc.is_empty());
    }

    #[test]
    fn accumulator_skips_nameless_slots() {
        let mut acc = ToolCallAccumulator::new();
        acc.push(0, Some("id"), None, "{}");
        assert!(acc.drain().is_empty());
    }
}
