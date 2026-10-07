//! Strict, bounded decoding of Pi's experimental child directory. The engine, not the UI,
//! owns execution and the durable enable setting; these rows are scoped to the attachment.

use pipkin_core::SubagentInfo;
use serde_json::Value;

const MAX_CHILDREN: usize = 4_096;
const MAX_TASK_CHARS: usize = 160;

pub(super) fn parse_directory(value: &Value) -> Result<Vec<SubagentInfo>, String> {
    let rows = value
        .as_array()
        .ok_or_else(|| "The engine returned an invalid subagent directory.".to_string())?;
    if rows.len() > MAX_CHILDREN {
        return Err("This session has too many subagents to display safely.".into());
    }
    rows.iter()
        .map(|row| {
            let id = row
                .get("conversationId")
                .and_then(Value::as_u64)
                .filter(|id| *id > 0 && *id <= 9_007_199_254_740_991)
                .ok_or("Invalid child identity")?;
            let task_id = row
                .get("taskId")
                .and_then(Value::as_u64)
                .filter(|id| *id > 0 && *id <= 9_007_199_254_740_991)
                .ok_or("Invalid owner task")?;
            let call_id = row
                .get("callId")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty() && s.len() <= 256)
                .ok_or("Invalid parent call")?;
            let task = row
                .get("task")
                .and_then(Value::as_str)
                .ok_or("Invalid child task")?;
            let status = row
                .get("status")
                .and_then(Value::as_str)
                .filter(|s| {
                    matches!(
                        *s,
                        "pending"
                            | "running"
                            | "waiting"
                            | "completing"
                            | "done"
                            | "failed"
                            | "aborted"
                    )
                })
                .ok_or("Invalid child status")?;
            Ok(SubagentInfo {
                id,
                task_id,
                call_id: call_id.to_owned(),
                task: task.chars().take(MAX_TASK_CHARS).collect(),
                status: status.to_owned(),
            })
        })
        .collect::<Result<Vec<_>, &str>>()
        .map_err(|error| format!("The engine returned an invalid subagent: {error}."))
}

pub(super) fn parse_enabled(value: &Value) -> Option<bool> {
    value.get("enabled").and_then(Value::as_bool)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn valid_children_are_bounded_and_status_is_checked() {
        let raw = json!([{
            "conversationId": 3, "taskId": 8, "callId": "call-1",
            "task": "x".repeat(1000), "status": "waiting"
        }]);
        let list = parse_directory(&raw).unwrap();
        assert_eq!(
            (list[0].id, list[0].task_id, list[0].call_id.as_str()),
            (3, 8, "call-1")
        );
        assert_eq!(list[0].task.chars().count(), MAX_TASK_CHARS);
        assert_eq!(list[0].status, "waiting");
        assert!(parse_directory(&json!([{"conversationId": -1}])).is_err());
        assert!(parse_directory(&json!([{"conversationId": 3, "taskId": 8, "callId": "c", "task": "a", "status": "unknown"}])).is_err());
        assert!(parse_directory(&Value::Array(vec![Value::Null; MAX_CHILDREN + 1])).is_err());
        assert_eq!(parse_enabled(&json!({"enabled": false})), Some(false));
        assert_eq!(parse_enabled(&json!({"enabled": "false"})), None);
    }
}
