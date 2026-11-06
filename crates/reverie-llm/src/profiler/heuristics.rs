//! Static capability heuristics.
//!
//! Probing gives ground truth but costs a round-trip per capability per model,
//! and many providers rate-limit aggressive probing. These heuristics provide a
//! sensible prior from the model name, family and provider metadata; the probe
//! and the evaluation agent then correct it.

use reverie_core::domain::{CostTier, ModelProfile, Protocol};
use reverie_core::llm::{RemoteModel, ThinkingLevel};
use reverie_core::time;

/// A prior about what a model can do, before any live probe.
#[derive(Debug, Clone)]
pub struct InferredCapabilities {
    pub context_window: i64,
    pub supports_vision: bool,
    pub supports_tools: bool,
    pub supports_thinking: bool,
    pub supported_thinking_levels: Vec<ThinkingLevel>,
    pub reasoning_score: f64,
    pub coding_score: f64,
    pub extraction_score: f64,
    pub cost_tier: CostTier,
    pub notes: String,
}

fn name_contains_any(name: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| name.contains(needle))
}

/// Families known to accept image input.
const VISION_HINTS: [&str; 24] = [
    "vision",
    "gpt-4o",
    "gpt-4.1",
    "gpt-5",
    "o3",
    "o4",
    "claude-3",
    "claude-4",
    "claude-sonnet",
    "claude-opus",
    "claude-haiku",
    "gemini",
    "llava",
    "bakllava",
    "qwen-vl",
    "qwen2-vl",
    "qwen2.5-vl",
    "qwen3-vl",
    "pixtral",
    "internvl",
    "minicpm-v",
    "moondream",
    "glm-4v",
    "phi-4-multimodal",
];

/// Families that expose a reasoning effort / thinking budget knob.
const THINKING_HINTS: [&str; 18] = [
    "o1",
    "o3",
    "o4",
    "gpt-5",
    "r1",
    "reasoner",
    "reasoning",
    "thinking",
    "qwq",
    "claude-3-7",
    "claude-sonnet-4",
    "claude-opus-4",
    "gemini-2.5",
    "gemini-3",
    "deepseek-v3.1",
    "magistral",
    "qwen3",
    "glm-4.5",
];

const CHEAP_HINTS: [&str; 14] =
    ["mini", "nano", "flash", "lite", "small", "haiku", "instant", "1b", "3b", "4b", "7b", "8b", "9b", "12b"];

const FRONTIER_HINTS: [&str; 11] =
    ["opus", "pro", "ultra", "large", "70b", "72b", "235b", "405b", "gpt-5", "o3", "max"];

/// Infer capabilities from a discovered model record.
pub fn infer(remote: &RemoteModel, protocol: Protocol) -> InferredCapabilities {
    let name = remote.model_name.to_ascii_lowercase();
    let hints_text = remote.hints.to_string().to_ascii_lowercase();

    let supports_vision = name_contains_any(&name, &VISION_HINTS) || hints_text.contains("\"vision\"");
    let supports_thinking = name_contains_any(&name, &THINKING_HINTS)
        || hints_text.contains("\"thinking\"")
        || hints_text.contains("\"reasoning\"");

    let context_window = remote
        .context_window
        .filter(|value| *value > 0)
        .unwrap_or_else(|| default_context_window(&name, protocol));

    let cheap = name_contains_any(&name, &CHEAP_HINTS);
    let frontier = name_contains_any(&name, &FRONTIER_HINTS);

    let cost_tier = if cheap && !frontier {
        CostTier::new(1)
    } else if frontier && !cheap {
        CostTier::new(3)
    } else {
        CostTier::new(2)
    };

    let (mut reasoning, mut coding, mut extraction): (f64, f64, f64) = match cost_tier.0 {
        1 => (5.6, 5.4, 7.6),
        3 => (9.0, 8.8, 6.0),
        _ => (7.0, 7.0, 6.6),
    };

    if supports_thinking {
        reasoning += 0.5;
    }
    if name_contains_any(&name, &["coder", "code", "devstral", "starcoder"]) {
        coding += 0.7;
    }
    if name_contains_any(&name, &["flash", "mini", "turbo"]) {
        extraction += 0.4;
    }
    if supports_vision {
        extraction += 0.2;
    }

    let levels = if supports_thinking {
        ThinkingLevel::all().to_vec()
    } else {
        vec![ThinkingLevel::Off]
    };

    let notes = compose_notes(&remote.model_name, cost_tier, supports_vision, supports_thinking, context_window);

    InferredCapabilities {
        context_window,
        supports_vision,
        supports_tools: true,
        supports_thinking,
        supported_thinking_levels: levels,
        reasoning_score: reasoning.clamp(0.0, 10.0),
        coding_score: coding.clamp(0.0, 10.0),
        extraction_score: extraction.clamp(0.0, 10.0),
        cost_tier,
        notes,
    }
}

/// Conventional context window per family when the provider does not report one.
pub fn default_context_window(name: &str, protocol: Protocol) -> i64 {
    if name.contains("gemini") {
        return 1_048_576;
    }
    if name.contains("claude") {
        return 200_000;
    }
    if name.contains("gpt-5") {
        return 400_000;
    }
    if name.contains("gpt-4.1") || name.contains("gpt-4o") {
        return 128_000;
    }
    if name.starts_with("o1") || name.starts_with("o3") || name.starts_with("o4") {
        return 200_000;
    }
    if name.contains("deepseek") {
        return 128_000;
    }
    if name.contains("llama3.1") || name.contains("llama3.2") || name.contains("llama3.3") || name.contains("llama4") {
        return 131_072;
    }
    if name.contains("qwen") {
        return 32_768;
    }
    if name.contains("mistral") || name.contains("mixtral") {
        return 32_768;
    }
    match protocol {
        Protocol::Ollama => 8_192,
        _ => 32_768,
    }
}

fn compose_notes(
    model: &str,
    tier: CostTier,
    vision: bool,
    thinking: bool,
    context: i64,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(match tier.0 {
        1 => "Low-cost worker: cheap enough to fan out across hundreds of parallel sub-agents.".to_string(),
        3 => "Frontier model: reserve for architecture, hard debugging and final synthesis.".to_string(),
        _ => "Balanced generalist: good default for mixed workloads.".to_string(),
    });
    if thinking {
        parts.push("Exposes a reasoning-effort knob, so thinking depth can be dialled per task.".to_string());
    } else {
        parts.push("No reasoning mode: keep thinking depth off and rely on decomposition.".to_string());
    }
    if vision {
        parts.push("Accepts image input, suitable for UI screenshots and diagram inspection.".to_string());
    } else {
        parts.push("Text only: image attachments must be described or OCR'd before use.".to_string());
    }
    parts.push(format!("Reported context window ≈ {} tokens.", context));
    format!("{model}: {}", parts.join(" "))
}

/// Build a fresh profile row from heuristics alone.
pub fn to_profile(provider_id: &str, remote: &RemoteModel, protocol: Protocol) -> ModelProfile {
    let inferred = infer(remote, protocol);
    ModelProfile {
        id: ModelProfile::compose_id(provider_id, &remote.model_name),
        provider_id: provider_id.to_string(),
        model_name: remote.model_name.clone(),
        context_window: inferred.context_window,
        supports_tools: inferred.supports_tools,
        supports_vision: inferred.supports_vision,
        supports_thinking: inferred.supports_thinking,
        default_thinking_level: if inferred.supports_thinking {
            ThinkingLevel::Medium
        } else {
            ThinkingLevel::Off
        },
        supported_thinking_levels: inferred.supported_thinking_levels,
        latency_ms: 1000,
        reasoning_score: inferred.reasoning_score,
        coding_score: inferred.coding_score,
        extraction_score: inferred.extraction_score,
        cost_tier: inferred.cost_tier,
        total_calls: 0,
        success_calls: 0,
        task_fit_notes: inferred.notes,
        updated_at: time::now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vision_families_are_detected() {
        let inferred = infer(&RemoteModel::new("gpt-4o-mini"), Protocol::OpenAiCompat);
        assert!(inferred.supports_vision);
        assert_eq!(inferred.cost_tier.0, 1);
    }

    #[test]
    fn thinking_families_are_detected() {
        let inferred = infer(&RemoteModel::new("deepseek-r1:32b"), Protocol::Ollama);
        assert!(inferred.supports_thinking);
        assert_eq!(inferred.supported_thinking_levels.len(), 5);
    }

    #[test]
    fn plain_models_get_off_only() {
        let inferred = infer(&RemoteModel::new("llama3.1:8b"), Protocol::Ollama);
        assert_eq!(inferred.supported_thinking_levels, vec![ThinkingLevel::Off]);
        assert_eq!(inferred.cost_tier.0, 1);
    }

    #[test]
    fn frontier_models_get_tier_three() {
        let inferred = infer(&RemoteModel::new("claude-opus-4"), Protocol::Anthropic);
        assert_eq!(inferred.cost_tier.0, 3);
        assert!(inferred.supports_vision);
        assert!(inferred.supports_thinking);
    }

    #[test]
    fn context_defaults_follow_family() {
        assert_eq!(default_context_window("gemini-2.5-pro", Protocol::Gemini), 1_048_576);
        assert_eq!(default_context_window("claude-sonnet-4", Protocol::Anthropic), 200_000);
        assert_eq!(default_context_window("unknown-model", Protocol::Ollama), 8_192);
    }

    #[test]
    fn provider_reported_context_wins() {
        let mut remote = RemoteModel::new("mystery-model");
        remote.context_window = Some(999_999);
        assert_eq!(infer(&remote, Protocol::OpenAiCompat).context_window, 999_999);
    }

    #[test]
    fn profile_has_composite_id_and_notes() {
        let remote = RemoteModel::new("gpt-4.1-mini");
        let profile = to_profile("prov-1", &remote, Protocol::OpenAiCompat);
        assert_eq!(profile.id, "prov-1:gpt-4.1-mini");
        assert!(profile.task_fit_notes.contains("gpt-4.1-mini"));
        assert!(profile.supports_vision);
    }
}
