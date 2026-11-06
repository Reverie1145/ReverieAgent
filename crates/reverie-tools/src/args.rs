//! Typed, forgiving accessors for model-supplied JSON arguments.
//!
//! Models frequently emit numbers as strings or booleans as `"true"`, so these
//! helpers coerce rather than fail whenever the intent is unambiguous.

use serde_json::Value;

use crate::error::{ToolError, ToolResult};

/// Required string argument.
pub fn req_str(args: &Value, key: &str) -> ToolResult<String> {
    match args.get(key) {
        Some(Value::String(value)) => Ok(value.clone()),
        Some(Value::Number(value)) => Ok(value.to_string()),
        Some(Value::Bool(value)) => Ok(value.to_string()),
        Some(other) => Err(ToolError::invalid(format!("`{key}` must be a string, got {other}"))),
        None => Err(ToolError::invalid(format!("missing required argument `{key}`"))),
    }
}

/// Optional string argument; empty strings are treated as absent.
pub fn opt_str(args: &Value, key: &str) -> Option<String> {
    match args.get(key) {
        Some(Value::String(value)) if !value.trim().is_empty() => Some(value.clone()),
        Some(Value::Number(value)) => Some(value.to_string()),
        _ => None,
    }
}

/// Required integer argument.
pub fn req_i64(args: &Value, key: &str) -> ToolResult<i64> {
    match args.get(key) {
        Some(Value::Number(value)) => value
            .as_i64()
            .or_else(|| value.as_f64().map(|float| float as i64))
            .ok_or_else(|| ToolError::invalid(format!("`{key}` is not an integer"))),
        Some(Value::String(value)) => value
            .trim()
            .parse::<i64>()
            .map_err(|_| ToolError::invalid(format!("`{key}` must be an integer"))),
        Some(other) => Err(ToolError::invalid(format!("`{key}` must be an integer, got {other}"))),
        None => Err(ToolError::invalid(format!("missing required argument `{key}`"))),
    }
}

/// Optional integer argument with a default.
pub fn opt_i64(args: &Value, key: &str, default: i64) -> i64 {
    req_i64(args, key).unwrap_or(default)
}

/// Optional integer clamped into a range.
pub fn opt_i64_clamped(args: &Value, key: &str, default: i64, min: i64, max: i64) -> i64 {
    opt_i64(args, key, default).clamp(min, max)
}

/// Optional boolean argument.
pub fn opt_bool(args: &Value, key: &str, default: bool) -> bool {
    match args.get(key) {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) => matches!(value.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes"),
        Some(Value::Number(value)) => value.as_i64().unwrap_or(0) != 0,
        _ => default,
    }
}

/// Optional array of strings.
pub fn opt_string_array(args: &Value, key: &str) -> Vec<String> {
    match args.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| match item {
                Value::String(value) => Some(value.clone()),
                Value::Number(value) => Some(value.to_string()),
                _ => None,
            })
            .collect(),
        Some(Value::String(value)) => vec![value.clone()],
        _ => Vec::new(),
    }
}

/// Optional object argument.
pub fn opt_object(args: &Value, key: &str) -> serde_json::Map<String, Value> {
    match args.get(key) {
        Some(Value::Object(map)) => map.clone(),
        _ => serde_json::Map::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn required_strings_coerce_numbers() {
        assert_eq!(req_str(&json!({"a": "x"}), "a").unwrap(), "x");
        assert_eq!(req_str(&json!({"a": 3}), "a").unwrap(), "3");
        assert!(req_str(&json!({}), "a").is_err());
        assert!(req_str(&json!({"a": [1]}), "a").is_err());
    }

    #[test]
    fn optional_strings_ignore_blanks() {
        assert_eq!(opt_str(&json!({"a": "  "}), "a"), None);
        assert_eq!(opt_str(&json!({"a": 7}), "a"), Some("7".into()));
    }

    #[test]
    fn integers_parse_from_strings() {
        assert_eq!(req_i64(&json!({"n": "42"}), "n").unwrap(), 42);
        assert_eq!(req_i64(&json!({"n": 4.7}), "n").unwrap(), 4);
        assert!(req_i64(&json!({"n": "abc"}), "n").is_err());
        assert_eq!(opt_i64(&json!({}), "n", 9), 9);
    }

    #[test]
    fn clamping_applies() {
        assert_eq!(opt_i64_clamped(&json!({"n": 500}), "n", 10, 1, 30), 30);
        assert_eq!(opt_i64_clamped(&json!({}), "n", 10, 1, 30), 10);
    }

    #[test]
    fn booleans_accept_common_spellings() {
        assert!(opt_bool(&json!({"b": "TRUE"}), "b", false));
        assert!(opt_bool(&json!({"b": 1}), "b", false));
        assert!(!opt_bool(&json!({}), "b", false));
    }

    #[test]
    fn arrays_and_objects_are_lenient() {
        assert_eq!(opt_string_array(&json!({"a": ["x", 2]}), "a"), vec!["x".to_string(), "2".to_string()]);
        assert_eq!(opt_string_array(&json!({"a": "solo"}), "a"), vec!["solo".to_string()]);
        assert!(opt_string_array(&json!({}), "a").is_empty());
        assert_eq!(opt_object(&json!({"o": {"k": 1}}), "o").len(), 1);
        assert!(opt_object(&json!({}), "o").is_empty());
    }
}
