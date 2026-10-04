//! Pi sessions as Pipkin conversations: stable local ids, titles, and the catalogue built from
//! the replicated `pi.session-directory` state. Pi's session summaries carry only server id,
//! session id and creation time today, so titles are derived and there is one placeholder
//! project; real titles and working directories need the metadata contract on the Pi side.

use std::collections::{HashMap, HashSet};

use pipkin_core::{Bootstrap, ConversationId, ModelInfo, Project, ProjectId};
use serde_json::Value;

pub const SESSIONS_PROJECT: ProjectId = ProjectId(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub session_id: String,
    /// Unix seconds.
    pub created_at: i64,
}

/// FNV-1a over the session id, folded into 52 bits so it survives JSON and stays a stable key.
fn hash(session_id: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in session_id.bytes() {
        h ^= byte as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    (h & ((1 << 52) - 1)).max(1)
}

/// Assign each session a stable `ConversationId` derived from its id. A collision (two live
/// sessions hashing alike) is resolved by probing in a deterministic order, so the same set of
/// sessions always gets the same ids.
pub fn assign_ids_with(
    sessions: &[Session],
    hash: impl Fn(&str) -> u64,
) -> Vec<(ConversationId, Session)> {
    let mut ordered: Vec<&Session> = sessions.iter().collect();
    ordered.sort_by(|a, b| (a.created_at, &a.session_id).cmp(&(b.created_at, &b.session_id)));
    let mut taken = HashSet::new();
    let mut out = Vec::new();
    for session in ordered {
        let mut id = hash(&session.session_id);
        while !taken.insert(id) {
            id = id.wrapping_add(1).max(1);
        }
        out.push((ConversationId(id), session.clone()));
    }
    out
}

pub fn assign_ids(sessions: &[Session]) -> Vec<(ConversationId, Session)> {
    assign_ids_with(sessions, hash)
}

/// Parse the directory state value, ignoring malformed rows rather than failing the catalogue.
pub fn parse_directory(state: &Value) -> Vec<Session> {
    state
        .get("sessions")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let session_id = row.get("sessionId")?.as_str().filter(|s| !s.is_empty())?;
                    let created = row.get("createdAt").and_then(Value::as_i64).unwrap_or(0);
                    // Millisecond timestamps are far above any plausible second count.
                    let created_at = if created > 100_000_000_000 {
                        created / 1000
                    } else {
                        created
                    };
                    Some(Session {
                        session_id: session_id.to_owned(),
                        created_at,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `YYYY-MM-DD HH:MM` in UTC, without a date library.
pub fn format_time(unix_seconds: i64) -> String {
    let days = unix_seconds.div_euclid(86_400);
    let secs = unix_seconds.rem_euclid(86_400);
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        secs / 3600,
        secs % 3600 / 60
    )
}

pub fn title(session: &Session) -> String {
    let short: String = session.session_id.chars().take(8).collect();
    format!("Session {short} \u{b7} {}", format_time(session.created_at))
}

/// The catalogue the core consumes. Sessions are listed newest first by recency.
pub fn catalog(
    sessions: &[(ConversationId, Session)],
    models: Vec<ModelInfo>,
    location: &str,
    now: i64,
) -> Bootstrap {
    Bootstrap {
        projects: vec![Project {
            id: SESSIONS_PROJECT,
            name: "Pi sessions".into(),
            path: location.into(),
        }],
        models,
        conversations: sessions
            .iter()
            .map(|(id, s)| (*id, SESSIONS_PROJECT, title(s), s.created_at))
            .collect(),
        now,
    }
}

/// Map `pi.models` state. The configured model comes first so it is the default selection.
pub fn parse_models(state: &Value) -> Vec<ModelInfo> {
    let configured = state
        .get("configuration")
        .and_then(|c| c.get("model"))
        .filter(|m| m.is_object())
        .and_then(|m| Some((m.get("provider")?.as_str()?, m.get("modelId")?.as_str()?)))
        .map(|(p, m)| format!("{p}/{m}"));
    let mut models: Vec<ModelInfo> = state
        .get("catalog")
        .and_then(|c| c.get("availableModels"))
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let provider = row.get("provider")?.as_str()?;
                    let model_id = row.get("modelId")?.as_str()?;
                    let name = row.get("name").and_then(Value::as_str).unwrap_or(model_id);
                    let reasoning = row
                        .get("reasoning")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    Some(ModelInfo {
                        id: format!("{provider}/{model_id}"),
                        name: name.to_owned(),
                        note: if reasoning {
                            format!("{provider} \u{b7} reasoning")
                        } else {
                            provider.to_owned()
                        },
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if let Some(configured) = configured
        && let Some(at) = models.iter().position(|m| m.id == configured)
    {
        let model = models.remove(at);
        models.insert(0, model);
    }
    models
}

/// Reverse index from local conversation id to session id.
pub fn index(sessions: &[(ConversationId, Session)]) -> HashMap<ConversationId, String> {
    sessions
        .iter()
        .map(|(id, s)| (*id, s.session_id.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn s(id: &str, at: i64) -> Session {
        Session {
            session_id: id.into(),
            created_at: at,
        }
    }

    #[test]
    fn ids_are_stable_for_a_session_regardless_of_the_others() {
        let a = assign_ids(&[s("alpha", 1)]);
        let b = assign_ids(&[s("zeta", 0), s("alpha", 1), s("mid", 5)]);
        let id_of = |v: &[(ConversationId, Session)], name: &str| {
            v.iter().find(|(_, x)| x.session_id == name).unwrap().0
        };
        assert_eq!(id_of(&a, "alpha"), id_of(&b, "alpha"));
        assert!(b.iter().all(|(id, _)| id.0 > 0 && id.0 < (1 << 52)));
    }

    #[test]
    fn collisions_probe_deterministically() {
        let constant = |_: &str| 7u64;
        let first = assign_ids_with(&[s("b", 2), s("a", 1), s("c", 3)], constant);
        let ids: Vec<u64> = first.iter().map(|(i, _)| i.0).collect();
        assert_eq!(ids, [7, 8, 9]);
        // The oldest session keeps the base id however the input is ordered.
        let again = assign_ids_with(&[s("c", 3), s("a", 1), s("b", 2)], constant);
        assert_eq!(first, again);
        assert_eq!(first[0].1.session_id, "a");
    }

    #[test]
    fn parses_directory_rows_and_ignores_malformed_ones() {
        let state = json!({ "revision": 2, "sessions": [
            { "serverId": "x", "sessionId": "one", "createdAt": 1_700_000_000_000i64 },
            { "sessionId": "two", "createdAt": 1_700_000_001 },
            { "sessionId": "", "createdAt": 1 },
            { "createdAt": 1 },
            7,
        ]});
        let rows = parse_directory(&state);
        assert_eq!(rows, vec![s("one", 1_700_000_000), s("two", 1_700_000_001)]);
        assert!(parse_directory(&json!(null)).is_empty());
        assert!(parse_directory(&json!({ "sessions": "no" })).is_empty());
    }

    #[test]
    fn formats_utc_times() {
        assert_eq!(format_time(0), "1970-01-01 00:00");
        assert_eq!(format_time(1_700_000_000), "2023-11-14 22:13");
        assert_eq!(format_time(951_782_400), "2000-02-29 00:00"); // leap day
        assert_eq!(format_time(-1), "1969-12-31 23:59");
    }

    #[test]
    fn catalogue_has_one_honest_project_and_derived_titles() {
        let sessions = assign_ids(&[s("0123456789abcdef", 1_700_000_000)]);
        let boot = catalog(&sessions, vec![], "/home/u/.pi/server", 5);
        assert_eq!(boot.projects.len(), 1);
        assert_eq!(boot.projects[0].path, "/home/u/.pi/server");
        assert_eq!(
            boot.conversations[0].2,
            "Session 01234567 \u{b7} 2023-11-14 22:13"
        );
        assert_eq!(
            index(&sessions).get(&sessions[0].0).unwrap(),
            "0123456789abcdef"
        );
    }

    #[test]
    fn models_put_the_configured_one_first() {
        let state = json!({
            "catalog": { "revision": 1, "availableModels": [
                { "provider": "anthropic", "modelId": "a", "name": "A", "reasoning": false },
                { "provider": "openai", "modelId": "b", "name": "B", "reasoning": true },
                { "provider": "x" },
            ]},
            "configuration": { "model": { "provider": "openai", "modelId": "b" }, "thinkingLevel": "off" },
        });
        let models = parse_models(&state);
        assert_eq!(
            models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["openai/b", "anthropic/a"]
        );
        assert!(models[0].note.contains("reasoning"));
        assert!(parse_models(&json!({})).is_empty());
        // A configured model absent from the catalogue changes nothing.
        let absent = json!({ "catalog": { "availableModels": [{ "provider": "p", "modelId": "m", "name": "M" }] },
            "configuration": { "model": { "provider": "q", "modelId": "z" } } });
        assert_eq!(parse_models(&absent).len(), 1);
    }
}
