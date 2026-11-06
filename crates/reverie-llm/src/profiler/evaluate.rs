//! Capability evaluation agent: one LLM call that turns raw probe data into
//! structured quality scores and task-fit notes.

use reverie_core::domain::Provider;
use reverie_core::llm::{ChatMessage, LlmRequest, ThinkingLevel};
use serde::{Deserialize, Serialize};

use crate::error::LlmResult;
use crate::gateway::LlmGateway;

/// One model's evaluated capabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationScore {
    pub model_name: String,
    pub reasoning_score: f64,
    pub coding_score: f64,
    pub extraction_score: f64,
    pub cost_tier: i64,
    pub task_fit_notes: String,
}

impl EvaluationScore {
    /// Clamp every numeric field into its documented range.
    pub fn sanitized(mut self) -> Self {
        self.reasoning_score = self.reasoning_score.clamp(0.0, 10.0);
        self.coding_score = self.coding_score.clamp(0.0, 10.0);
        self.extraction_score = self.extraction_score.clamp(0.0, 10.0);
        self.cost_tier = self.cost_tier.clamp(1, 3);
        if self.task_fit_notes.trim().is_empty() {
            self.task_fit_notes = format!("{}: evaluated by the profiler.", self.model_name);
        }
        self
    }
}

/// What the evaluation agent is told about each model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationInput {
    pub model_name: String,
    pub context_window: i64,
    pub supports_vision: bool,
    pub supports_thinking: bool,
    pub latency_ms: i64,
    pub thinking_levels: Vec<String>,
}

/// Extract the first balanced JSON array from free-form model output.
pub fn extract_json_array(text: &str) -> Option<&str> {
    let start = text.find('[')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, ch) in text[start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    let end = start + offset + ch.len_utf8();
                    return text.get(start..end);
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse an evaluation reply, tolerating prose around the JSON payload.
pub fn parse_scores(text: &str) -> Vec<EvaluationScore> {
    if let Some(json) = extract_json_array(text) {
        if let Ok(scores) = serde_json::from_str::<Vec<EvaluationScore>>(json) {
            return scores.into_iter().map(EvaluationScore::sanitized).collect();
        }
    }
    // Some models answer with a single object rather than an array.
    if let Some(json) = extract_json_object(text) {
        if let Ok(score) = serde_json::from_str::<EvaluationScore>(json) {
            return vec![score.sanitized()];
        }
    }
    Vec::new()
}

fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, ch) in text[start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let end = start + offset + ch.len_utf8();
                    return text.get(start..end);
                }
            }
            _ => {}
        }
    }
    None
}

/// The prompt handed to the evaluation model.
pub fn build_prompt(inputs: &[EvaluationInput]) -> String {
    let roster = serde_json::to_string_pretty(inputs).unwrap_or_else(|_| "[]".to_string());
    format!(
        "You are the Model Profiler for an autonomous multi-agent system.\n\
         Below is the live capability roster discovered from one LLM provider.\n\n\
         {roster}\n\n\
         Score every model on a 0.0–10.0 scale for:\n\
         - reasoning_score: architecture design, long-horizon planning, hard debugging.\n\
         - coding_score: writing and refactoring code, reading compiler errors.\n\
         - extraction_score: high-throughput scraping, summarisation and data cleaning.\n\
         Also assign cost_tier: 1 = cheap high-concurrency worker, 2 = balanced, 3 = frontier flagship.\n\
         Finally write task_fit_notes: 1–3 sentences on strengths, weaknesses, and the ideal tasks.\n\n\
         Reply with ONLY a JSON array. Each element must be:\n\
         {{\"model_name\": string, \"reasoning_score\": number, \"coding_score\": number, \
         \"extraction_score\": number, \"cost_tier\": 1|2|3, \"task_fit_notes\": string}}\n\
         Include every model from the roster exactly once. No prose, no markdown fences."
    )
}

/// Ask the evaluator model to score the roster.
pub async fn evaluate_models(
    gateway: &LlmGateway,
    provider: &Provider,
    evaluator_model: &str,
    inputs: &[EvaluationInput],
) -> LlmResult<Vec<EvaluationScore>> {
    if inputs.is_empty() {
        return Ok(Vec::new());
    }
    let request = LlmRequest::new(
        evaluator_model,
        vec![
            ChatMessage::system(
                "You are a precise model-capability evaluator. You always answer with strict JSON.",
            ),
            ChatMessage::user(build_prompt(inputs)),
        ],
    )
    .with_thinking(ThinkingLevel::Medium)
    .with_limits(Some(4096), Some(0.1));

    let completion = gateway.complete_with_retry(provider, request).await?;
    Ok(parse_scores(&completion.text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn array_extraction_ignores_prose() {
        let text = "Sure! Here you go:\n[{\"a\":1},{\"a\":2}]\nHope that helps.";
        assert_eq!(extract_json_array(text), Some("[{\"a\":1},{\"a\":2}]"));
    }

    #[test]
    fn extraction_respects_strings_with_brackets() {
        let text = "[[1] inside a string is fine]".to_string();
        let wrapped = format!("prefix [\"{text}\"] suffix");
        assert_eq!(extract_json_array(&wrapped), Some(&*format!("[\"{text}\"]")));
    }

    #[test]
    fn parse_scores_reads_array() {
        let text = r#"[
            {"model_name":"a","reasoning_score":9.5,"coding_score":8,"extraction_score":4,"cost_tier":3,"task_fit_notes":"flagship"},
            {"model_name":"b","reasoning_score":3,"coding_score":4,"extraction_score":9,"cost_tier":1,"task_fit_notes":"worker"}
        ]"#;
        let scores = parse_scores(text);
        assert_eq!(scores.len(), 2);
        assert_eq!(scores[0].model_name, "a");
        assert_eq!(scores[1].cost_tier, 1);
    }

    #[test]
    fn parse_scores_reads_single_object() {
        let text = r#"{"model_name":"solo","reasoning_score":7,"coding_score":7,"extraction_score":7,"cost_tier":2,"task_fit_notes":"ok"}"#;
        let scores = parse_scores(text);
        assert_eq!(scores.len(), 1);
        assert_eq!(scores[0].model_name, "solo");
    }

    #[test]
    fn parse_scores_clamps_out_of_range_values() {
        let text = r#"[{"model_name":"x","reasoning_score":99,"coding_score":-4,"extraction_score":5,"cost_tier":9,"task_fit_notes":""}]"#;
        let scores = parse_scores(text);
        assert_eq!(scores[0].reasoning_score, 10.0);
        assert_eq!(scores[0].coding_score, 0.0);
        assert_eq!(scores[0].cost_tier, 3);
        assert!(scores[0].task_fit_notes.contains('x'));
    }

    #[test]
    fn garbage_yields_empty() {
        assert!(parse_scores("I cannot help with that.").is_empty());
    }

    #[test]
    fn prompt_lists_every_model() {
        let inputs = vec![EvaluationInput {
            model_name: "m1".into(),
            context_window: 1000,
            supports_vision: true,
            supports_thinking: false,
            latency_ms: 120,
            thinking_levels: vec!["off".into()],
        }];
        let prompt = build_prompt(&inputs);
        assert!(prompt.contains("m1"));
        assert!(prompt.contains("reasoning_score"));
    }
}
