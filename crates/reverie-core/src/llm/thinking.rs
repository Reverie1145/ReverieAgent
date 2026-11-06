//! Thinking depth, the single cross-provider abstraction for reasoning effort.

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

/// How hard a model should think before answering.
///
/// The value is protocol agnostic; each adapter maps it onto the provider's
/// native knob (`reasoning_effort`, `thinking.budget_tokens`,
/// `generationConfig.thinkingConfig`, Ollama's `think` flag, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingLevel {
    Off,
    Low,
    Medium,
    High,
    Max,
}

impl Default for ThinkingLevel {
    fn default() -> Self {
        ThinkingLevel::Medium
    }
}

impl ThinkingLevel {
    /// Canonical wire/DB representation.
    pub fn as_str(self) -> &'static str {
        match self {
            ThinkingLevel::Off => "off",
            ThinkingLevel::Low => "low",
            ThinkingLevel::Medium => "medium",
            ThinkingLevel::High => "high",
            ThinkingLevel::Max => "max",
        }
    }

    /// Parse a level, accepting the `auto` sentinel as `None` so callers can
    /// distinguish "let the router decide" from an explicit depth.
    pub fn parse_optional(value: &str) -> CoreResult<Option<Self>> {
        let normalized = value.trim().to_ascii_lowercase();
        if normalized.is_empty() || normalized == "auto" {
            return Ok(None);
        }
        ThinkingLevel::parse(&normalized).map(Some)
    }

    /// Strict parser used by the store and IPC boundary.
    pub fn parse(value: &str) -> CoreResult<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "disabled" => Ok(ThinkingLevel::Off),
            "low" | "minimal" | "light" => Ok(ThinkingLevel::Low),
            "medium" | "normal" | "default" => Ok(ThinkingLevel::Medium),
            "high" => Ok(ThinkingLevel::High),
            "max" | "maximum" | "extreme" => Ok(ThinkingLevel::Max),
            other => Err(CoreError::InvalidEnum { kind: "ThinkingLevel", value: other.to_string() }),
        }
    }

    /// All levels in ascending order.
    pub fn all() -> [ThinkingLevel; 5] {
        [
            ThinkingLevel::Off,
            ThinkingLevel::Low,
            ThinkingLevel::Medium,
            ThinkingLevel::High,
            ThinkingLevel::Max,
        ]
    }

    /// True when the model is expected to emit reasoning tokens.
    pub fn is_reasoning(self) -> bool {
        !matches!(self, ThinkingLevel::Off)
    }

    /// Anthropic extended-thinking budget in tokens.
    ///
    /// Anthropic requires `budget_tokens >= 1024`; `Off` means the block is
    /// omitted entirely, which the adapter handles by returning `None`.
    pub fn anthropic_budget(self) -> Option<u32> {
        match self {
            ThinkingLevel::Off => None,
            ThinkingLevel::Low => Some(2048),
            ThinkingLevel::Medium => Some(8192),
            ThinkingLevel::High => Some(16384),
            ThinkingLevel::Max => Some(32000),
        }
    }

    /// OpenAI `reasoning_effort` string, or `None` when reasoning is disabled.
    ///
    /// OpenAI only exposes three tiers, so `Max` collapses onto `high` and
    /// `Off` removes the field completely.
    pub fn openai_reasoning_effort(self) -> Option<&'static str> {
        match self {
            ThinkingLevel::Off => None,
            ThinkingLevel::Low => Some("low"),
            ThinkingLevel::Medium => Some("medium"),
            ThinkingLevel::High | ThinkingLevel::Max => Some("high"),
        }
    }

    /// Gemini `thinkingConfig.thinkingBudget`. `Off` maps to `0` (disable) and
    /// `Max` to `-1`, which Gemini documents as "dynamic maximum".
    pub fn gemini_thinking_budget(self) -> i32 {
        match self {
            ThinkingLevel::Off => 0,
            ThinkingLevel::Low => 1024,
            ThinkingLevel::Medium => 8192,
            ThinkingLevel::High => 24576,
            ThinkingLevel::Max => -1,
        }
    }

    /// Fraction of the context window a reasoning pass may consume. Used by the
    /// router when it has to cap `Max` for models with small windows.
    pub fn context_share(self) -> f64 {
        match self {
            ThinkingLevel::Off => 0.0,
            ThinkingLevel::Low => 0.08,
            ThinkingLevel::Medium => 0.20,
            ThinkingLevel::High => 0.35,
            ThinkingLevel::Max => 0.50,
        }
    }

    /// The highest level a model advertises, given its supported list.
    pub fn clamp_to(self, supported: &[ThinkingLevel]) -> ThinkingLevel {
        if supported.is_empty() {
            return ThinkingLevel::Off;
        }
        let mut best = ThinkingLevel::Off;
        for level in supported {
            if *level <= self && *level > best {
                best = *level;
            }
        }
        best
    }

    /// Parse a JSON array of level names (the `supported_thinking_levels` column).
    pub fn parse_list(json: &str) -> Vec<ThinkingLevel> {
        let raw: Vec<String> = serde_json::from_str(json).unwrap_or_default();
        let mut levels: Vec<ThinkingLevel> =
            raw.iter().filter_map(|item| ThinkingLevel::parse(item).ok()).collect();
        levels.sort();
        levels.dedup();
        if levels.is_empty() {
            levels.push(ThinkingLevel::Off);
        }
        levels
    }

    /// Render a level list back into the storage JSON form.
    pub fn encode_list(levels: &[ThinkingLevel]) -> String {
        let names: Vec<&str> = levels.iter().map(|level| level.as_str()).collect();
        serde_json::to_string(&names).unwrap_or_else(|_| "[\"off\"]".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_all_levels() {
        for level in ThinkingLevel::all() {
            assert_eq!(ThinkingLevel::parse(level.as_str()).unwrap(), level);
        }
    }

    #[test]
    fn auto_parses_to_none() {
        assert_eq!(ThinkingLevel::parse_optional("auto").unwrap(), None);
        assert_eq!(ThinkingLevel::parse_optional("").unwrap(), None);
        assert_eq!(ThinkingLevel::parse_optional("HIGH").unwrap(), Some(ThinkingLevel::High));
    }

    #[test]
    fn anthropic_budgets_are_monotonic() {
        let budgets: Vec<u32> = ThinkingLevel::all().iter().filter_map(|l| l.anthropic_budget()).collect();
        assert_eq!(budgets, vec![2048, 8192, 16384, 32000]);
    }

    #[test]
    fn clamping_respects_supported_set() {
        let supported = vec![ThinkingLevel::Off, ThinkingLevel::Low, ThinkingLevel::Medium];
        assert_eq!(ThinkingLevel::Max.clamp_to(&supported), ThinkingLevel::Medium);
        assert_eq!(ThinkingLevel::Low.clamp_to(&supported), ThinkingLevel::Low);
        assert_eq!(ThinkingLevel::High.clamp_to(&[]), ThinkingLevel::Off);
    }

    #[test]
    fn list_encoding_is_stable() {
        let encoded = ThinkingLevel::encode_list(&ThinkingLevel::all());
        assert_eq!(ThinkingLevel::parse_list(&encoded).len(), 5);
        assert_eq!(ThinkingLevel::parse_list("not json"), vec![ThinkingLevel::Off]);
    }
}
