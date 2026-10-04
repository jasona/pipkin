//! Routed envelopes for protocol version 8, validated strictly: unknown properties are
//! rejected, ids are non-empty, server ids are canonical lowercase UUIDv4, and every opaque
//! payload (`call`, `result`, `update`) is plain JSON.

use serde_json::{Map, Value, json};

use crate::cbor::{self, Limits};
use crate::error::{Error, Result, validation};
use crate::frame::{DEFAULT_MAX_FRAME_LENGTH, FrameDecoder, encode_frame};

pub const PROTOCOL_VERSION: u64 = 8;

/// Canonical lowercase UUIDv4: `xxxxxxxx-xxxx-4xxx-[89ab]xxx-xxxxxxxxxxxx`.
pub fn is_server_id(value: &str) -> bool {
    let b = value.as_bytes();
    if b.len() != 36 {
        return false;
    }
    for (i, c) in b.iter().enumerate() {
        let ok = match i {
            8 | 13 | 18 | 23 => *c == b'-',
            14 => *c == b'4',
            19 => matches!(c, b'8' | b'9' | b'a' | b'b'),
            _ => matches!(c, b'0'..=b'9' | b'a'..=b'f'),
        };
        if !ok {
            return false;
        }
    }
    true
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtocolError {
    pub code: String,
    pub message: String,
}

/// A call routed to the server, or fenced to one live Session attachment.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RpcTarget {
    Server {
        server_id: String,
    },
    Session {
        server_id: String,
        session_id: String,
        attachment_id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SessionTarget {
    pub server_id: String,
    pub session_id: String,
    pub attachment_id: String,
}

impl SessionTarget {
    pub fn rpc(&self) -> RpcTarget {
        RpcTarget::Session {
            server_id: self.server_id.clone(),
            session_id: self.session_id.clone(),
            attachment_id: self.attachment_id.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ClientMessage {
    Hello {
        version: u64,
    },
    Request {
        id: String,
        target: RpcTarget,
        call: Value,
    },
    Cancel {
        id: String,
        target: RpcTarget,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ServerMessage {
    Hello {
        server_id: String,
    },
    HelloError {
        error: ProtocolError,
    },
    /// `result` is `None` for a void response that carries no `result` property.
    Response {
        id: String,
        outcome: std::result::Result<Option<Value>, ProtocolError>,
    },
    ServiceUpdate {
        subscription_id: String,
        update: Value,
    },
    Attachment(Option<SessionTarget>),
}

// ----------------------------------------------------------------------------- parsing

fn object<'a>(value: &'a Value, what: &str) -> Result<&'a Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| validation(format!("{what} is not an object")))
}

/// Require exactly `required` plus any subset of `optional`.
fn keys(map: &Map<String, Value>, required: &[&str], optional: &[&str], what: &str) -> Result<()> {
    for key in required {
        if !map.contains_key(*key) {
            return Err(validation(format!("{what} is missing {key:?}")));
        }
    }
    for key in map.keys() {
        if !required.contains(&key.as_str()) && !optional.contains(&key.as_str()) {
            return Err(validation(format!("{what} has unknown property {key:?}")));
        }
    }
    Ok(())
}

fn id(map: &Map<String, Value>, key: &str, what: &str) -> Result<String> {
    match map.get(key).and_then(Value::as_str) {
        Some(s) if !s.is_empty() => Ok(s.to_owned()),
        _ => Err(validation(format!(
            "{what}.{key} must be a non-empty string"
        ))),
    }
}

fn server_id(map: &Map<String, Value>, what: &str) -> Result<String> {
    match map.get("serverId").and_then(Value::as_str) {
        Some(s) if is_server_id(s) => Ok(s.to_owned()),
        _ => Err(validation(format!(
            "{what}.serverId must be a canonical UUIDv4"
        ))),
    }
}

fn parse_target(value: &Value) -> Result<RpcTarget> {
    let map = object(value, "target")?;
    if map.contains_key("sessionId") || map.contains_key("attachmentId") {
        keys(
            map,
            &["serverId", "sessionId", "attachmentId"],
            &[],
            "session target",
        )?;
        Ok(RpcTarget::Session {
            server_id: server_id(map, "target")?,
            session_id: id(map, "sessionId", "target")?,
            attachment_id: id(map, "attachmentId", "target")?,
        })
    } else {
        keys(map, &["serverId"], &[], "server target")?;
        Ok(RpcTarget::Server {
            server_id: server_id(map, "target")?,
        })
    }
}

fn parse_session_target(value: &Value) -> Result<SessionTarget> {
    match parse_target(value)? {
        RpcTarget::Session {
            server_id,
            session_id,
            attachment_id,
        } => Ok(SessionTarget {
            server_id,
            session_id,
            attachment_id,
        }),
        RpcTarget::Server { .. } => Err(validation("attachment must be a session target")),
    }
}

fn parse_error(value: &Value) -> Result<ProtocolError> {
    let map = object(value, "error")?;
    keys(map, &["code", "message"], &[], "error")?;
    Ok(ProtocolError {
        code: id(map, "code", "error")?,
        message: map
            .get("message")
            .and_then(Value::as_str)
            .ok_or_else(|| validation("error.message must be a string"))?
            .to_owned(),
    })
}

pub fn parse_client_message(value: &Value) -> Result<ClientMessage> {
    let map = object(value, "client message")?;
    match map.get("type").and_then(Value::as_str) {
        Some("hello") => {
            keys(map, &["type", "version"], &[], "client hello")?;
            let version = map
                .get("version")
                .and_then(Value::as_u64)
                .ok_or_else(|| validation("client hello version must be a non-negative integer"))?;
            Ok(ClientMessage::Hello { version })
        }
        Some("request") => {
            keys(map, &["type", "id", "target", "call"], &[], "request")?;
            Ok(ClientMessage::Request {
                id: id(map, "id", "request")?,
                target: parse_target(&map["target"])?,
                call: map["call"].clone(),
            })
        }
        Some("cancel") => {
            keys(map, &["type", "id", "target"], &[], "cancel")?;
            Ok(ClientMessage::Cancel {
                id: id(map, "id", "cancel")?,
                target: parse_target(&map["target"])?,
            })
        }
        _ => Err(validation("Invalid client protocol message")),
    }
}

pub fn parse_server_message(value: &Value) -> Result<ServerMessage> {
    let map = object(value, "server message")?;
    match map.get("type").and_then(Value::as_str) {
        Some("hello") => {
            keys(map, &["type", "version", "serverId"], &[], "server hello")?;
            if map["version"].as_u64() != Some(PROTOCOL_VERSION) {
                return Err(validation(format!(
                    "server hello version must be {PROTOCOL_VERSION}"
                )));
            }
            Ok(ServerMessage::Hello {
                server_id: server_id(map, "hello")?,
            })
        }
        Some("hello_error") => {
            keys(map, &["type", "error"], &[], "hello_error")?;
            Ok(ServerMessage::HelloError {
                error: parse_error(&map["error"])?,
            })
        }
        Some("response") => {
            let request_id = id(map, "id", "response")?;
            match map.get("ok").and_then(Value::as_bool) {
                Some(true) => {
                    keys(map, &["type", "id", "ok"], &["result"], "response")?;
                    Ok(ServerMessage::Response {
                        id: request_id,
                        outcome: Ok(map.get("result").cloned()),
                    })
                }
                Some(false) => {
                    keys(map, &["type", "id", "ok", "error"], &[], "response")?;
                    Ok(ServerMessage::Response {
                        id: request_id,
                        outcome: Err(parse_error(&map["error"])?),
                    })
                }
                None => Err(validation("response.ok must be a boolean")),
            }
        }
        Some("service_update") => {
            keys(
                map,
                &["type", "subscriptionId", "update"],
                &[],
                "service_update",
            )?;
            Ok(ServerMessage::ServiceUpdate {
                subscription_id: id(map, "subscriptionId", "service_update")?,
                update: map["update"].clone(),
            })
        }
        Some("attachment") => {
            keys(map, &["type", "attachment"], &[], "attachment")?;
            let attachment = match &map["attachment"] {
                Value::Null => None,
                other => Some(parse_session_target(other)?),
            };
            Ok(ServerMessage::Attachment(attachment))
        }
        _ => Err(validation("Invalid server protocol message")),
    }
}

// ----------------------------------------------------------------------------- encoding

fn target_value(target: &RpcTarget) -> Value {
    match target {
        RpcTarget::Server { server_id } => json!({ "serverId": server_id }),
        RpcTarget::Session {
            server_id,
            session_id,
            attachment_id,
        } => {
            json!({ "serverId": server_id, "sessionId": session_id, "attachmentId": attachment_id })
        }
    }
}

pub fn client_message_value(message: &ClientMessage) -> Value {
    match message {
        ClientMessage::Hello { version } => json!({ "type": "hello", "version": version }),
        ClientMessage::Request { id, target, call } => {
            json!({ "type": "request", "id": id, "target": target_value(target), "call": call })
        }
        ClientMessage::Cancel { id, target } => {
            json!({ "type": "cancel", "id": id, "target": target_value(target) })
        }
    }
}

pub fn server_message_value(message: &ServerMessage) -> Value {
    match message {
        ServerMessage::Hello { server_id } => {
            json!({ "type": "hello", "version": PROTOCOL_VERSION, "serverId": server_id })
        }
        ServerMessage::HelloError { error } => json!({
            "type": "hello_error",
            "error": { "code": error.code, "message": error.message },
        }),
        ServerMessage::Response {
            id,
            outcome: Ok(result),
        } => {
            let mut v = json!({ "type": "response", "id": id, "ok": true });
            if let Some(result) = result {
                v["result"] = result.clone();
            }
            v
        }
        ServerMessage::Response {
            id,
            outcome: Err(error),
        } => json!({
            "type": "response", "id": id, "ok": false,
            "error": { "code": error.code, "message": error.message },
        }),
        ServerMessage::ServiceUpdate {
            subscription_id,
            update,
        } => {
            json!({ "type": "service_update", "subscriptionId": subscription_id, "update": update })
        }
        ServerMessage::Attachment(None) => json!({ "type": "attachment", "attachment": null }),
        ServerMessage::Attachment(Some(t)) => json!({
            "type": "attachment",
            "attachment": target_value(&t.rpc()),
        }),
    }
}

fn limits_for(max_frame_length: usize) -> Limits {
    Limits {
        max_byte_length: max_frame_length,
        ..Limits::default()
    }
}

fn encode_message(value: &Value, max_frame_length: usize) -> Result<Vec<u8>> {
    let payload = cbor::encode(value, &limits_for(max_frame_length))
        .map_err(|e| validation(format!("Unable to encode protocol message: {e}")))?;
    encode_frame(&payload)
}

/// Validate and encode one complete length-prefixed client message.
pub fn encode_client_message(message: &ClientMessage, max_frame_length: usize) -> Result<Vec<u8>> {
    let value = client_message_value(message);
    parse_client_message(&value)?;
    encode_message(&value, max_frame_length)
}

/// Validate and encode one complete length-prefixed server message (used by test servers).
pub fn encode_server_message(message: &ServerMessage, max_frame_length: usize) -> Result<Vec<u8>> {
    let value = server_message_value(message);
    parse_server_message(&value)?;
    encode_message(&value, max_frame_length)
}

// ----------------------------------------------------------------------------- decoding

struct MessageDecoder<T> {
    frames: FrameDecoder,
    limits: Limits,
    parse: fn(&Value) -> Result<T>,
    kind: &'static str,
    failed: bool,
}

impl<T> MessageDecoder<T> {
    fn new(kind: &'static str, parse: fn(&Value) -> Result<T>, max_frame_length: usize) -> Self {
        MessageDecoder {
            frames: FrameDecoder::new(max_frame_length),
            limits: limits_for(max_frame_length),
            parse,
            kind,
            failed: false,
        }
    }

    fn push(&mut self, chunk: &[u8]) -> Result<Vec<T>> {
        if self.failed {
            return Err(validation(format!(
                "{} message decoder has failed",
                self.kind
            )));
        }
        let result = (|| {
            let mut messages = Vec::new();
            for frame in self.frames.push(chunk)? {
                let value = cbor::decode(&frame, &self.limits)?;
                messages.push((self.parse)(&value)?);
            }
            Ok(messages)
        })();
        if result.is_err() {
            self.failed = true;
        }
        result.map_err(|e| match e {
            Error::Validation(_) => e,
            other => validation(format!("Invalid {} protocol frame: {other}", self.kind)),
        })
    }

    fn end(&mut self) -> Result<()> {
        if self.failed {
            return Err(validation(format!(
                "{} message decoder has failed",
                self.kind
            )));
        }
        self.frames.end().map_err(|e| {
            self.failed = true;
            validation(format!("Invalid {} protocol framing: {e}", self.kind))
        })
    }
}

/// Incrementally decodes and validates framed server messages.
pub struct ServerMessageDecoder(MessageDecoder<ServerMessage>);

impl ServerMessageDecoder {
    pub fn new(max_frame_length: usize) -> Self {
        ServerMessageDecoder(MessageDecoder::new(
            "server",
            parse_server_message,
            max_frame_length,
        ))
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<ServerMessage>> {
        self.0.push(chunk)
    }

    pub fn end(&mut self) -> Result<()> {
        self.0.end()
    }
}

impl Default for ServerMessageDecoder {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_FRAME_LENGTH)
    }
}

/// Incrementally decodes and validates framed client messages (used by test servers).
pub struct ClientMessageDecoder(MessageDecoder<ClientMessage>);

impl ClientMessageDecoder {
    pub fn new(max_frame_length: usize) -> Self {
        ClientMessageDecoder(MessageDecoder::new(
            "client",
            parse_client_message,
            max_frame_length,
        ))
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<ClientMessage>> {
        self.0.push(chunk)
    }

    pub fn end(&mut self) -> Result<()> {
        self.0.end()
    }
}

impl Default for ClientMessageDecoder {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_FRAME_LENGTH)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVER: &str = "00000000-0000-4000-8000-000000000001";

    fn server_target() -> Value {
        json!({ "serverId": SERVER })
    }

    fn request(extra: Value) -> Value {
        let mut v = json!({
            "type": "request", "id": "request-1", "target": server_target(),
            "call": { "serviceId": "pi.models", "member": "list", "args": [] },
        });
        if let (Some(v), Some(extra)) = (v.as_object_mut(), extra.as_object()) {
            for (k, x) in extra {
                v.insert(k.clone(), x.clone());
            }
        }
        v
    }

    #[test]
    fn server_ids_must_be_canonical_uuid_v4() {
        assert!(is_server_id(SERVER));
        for bad in [
            "",
            "server-1",
            "00000000-0000-7000-8000-000000000001",
            "00000000-0000-4000-7000-000000000001",
            "00000000-0000-4000-8000-00000000000A",
        ] {
            assert!(!is_server_id(bad), "{bad:?}");
            let v = request(json!({ "target": { "serverId": bad } }));
            assert!(parse_client_message(&v).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn client_hello_accepts_any_integer_version_for_negotiation() {
        for version in [0u64, 8, 9] {
            let m = parse_client_message(&json!({ "type": "hello", "version": version })).unwrap();
            assert_eq!(m, ClientMessage::Hello { version });
        }
        for bad in [
            json!({ "type": "hello", "version": "8" }),
            json!({ "type": "hello", "version": 8.5 }),
            json!({ "type": "hello", "version": 8, "extra": true }),
        ] {
            assert!(parse_client_message(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn server_hello_must_be_version_8_with_a_valid_server_id() {
        let ok =
            parse_server_message(&json!({ "type": "hello", "version": 8, "serverId": SERVER }))
                .unwrap();
        assert_eq!(
            ok,
            ServerMessage::Hello {
                server_id: SERVER.into()
            }
        );
        for bad in [
            json!({ "type": "hello", "version": 7, "serverId": SERVER }),
            json!({ "type": "hello", "version": 8, "serverId": "server-1" }),
            json!({ "type": "hello", "version": 8, "serverId": SERVER, "snapshot": {} }),
        ] {
            assert!(parse_server_message(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn routed_payloads_stay_opaque_json() {
        let v = json!({
            "type": "request", "id": "request-1",
            "target": { "serverId": SERVER, "sessionId": "session-1", "attachmentId": "attachment-1" },
            "call": { "arbitrary": "strict JSON whose service meaning belongs to Chord" },
        });
        let ClientMessage::Request { target, call, .. } = parse_client_message(&v).unwrap() else {
            panic!()
        };
        assert!(matches!(target, RpcTarget::Session { .. }));
        assert_eq!(
            call["arbitrary"],
            "strict JSON whose service meaning belongs to Chord"
        );
        let update = parse_server_message(
            &json!({ "type": "service_update", "subscriptionId": "s", "update": { "applicationDefined": true } }),
        )
        .unwrap();
        assert!(matches!(update, ServerMessage::ServiceUpdate { .. }));
    }

    #[test]
    fn validates_cancellation_and_rejects_extra_or_empty_fields() {
        let cancel = json!({ "type": "cancel", "id": "request-1", "target": server_target() });
        assert!(parse_client_message(&cancel).is_ok());
        let mut empty = cancel.clone();
        empty["id"] = json!("");
        assert!(parse_client_message(&empty).is_err());
        let mut extra = cancel;
        extra["extra"] = json!(true);
        assert!(parse_client_message(&extra).is_err());
        assert!(parse_client_message(&request(json!({ "id": "" }))).is_err());
        assert!(parse_client_message(&request(json!({ "extra": true }))).is_err());
    }

    #[test]
    fn validates_attachment_route_updates() {
        let attached = json!({
            "type": "attachment",
            "attachment": { "serverId": SERVER, "sessionId": "session-1", "attachmentId": "attachment-1" },
        });
        let ServerMessage::Attachment(Some(t)) = parse_server_message(&attached).unwrap() else {
            panic!()
        };
        assert_eq!(
            (t.session_id.as_str(), t.attachment_id.as_str()),
            ("session-1", "attachment-1")
        );
        assert_eq!(
            parse_server_message(&json!({ "type": "attachment", "attachment": null })).unwrap(),
            ServerMessage::Attachment(None)
        );
        for bad in [
            json!({ "type": "attachment", "attachment": { "sessionId": "session-1" } }),
            // A server-only target is not an attachment.
            json!({ "type": "attachment", "attachment": { "serverId": SERVER } }),
        ] {
            assert!(parse_server_message(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_void_response_has_no_result_field() {
        let m = parse_server_message(&json!({ "type": "response", "id": "request-1", "ok": true }))
            .unwrap();
        assert_eq!(
            m,
            ServerMessage::Response {
                id: "request-1".into(),
                outcome: Ok(None)
            }
        );
        let null = parse_server_message(
            &json!({ "type": "response", "id": "r", "ok": true, "result": null }),
        )
        .unwrap();
        assert_eq!(
            null,
            ServerMessage::Response {
                id: "r".into(),
                outcome: Ok(Some(Value::Null))
            }
        );
        // The encoder preserves the distinction.
        assert!(server_message_value(&m).get("result").is_none());
        assert!(server_message_value(&null).get("result").is_some());
    }

    #[test]
    fn rejects_malformed_server_boundaries_and_accepts_opaque_error_codes() {
        assert!(
            parse_server_message(
                &json!({ "type": "response", "id": "r", "ok": true, "result": [], "extra": true })
            )
            .is_err()
        );
        assert!(parse_server_message(&json!({ "type": "response", "id": "r", "ok": false, "error": { "code": "", "message": "bad" } })).is_err());
        assert!(parse_server_message(&json!({ "type": "unknown", "event": {} })).is_err());
        for code in [
            "wrong_server",
            "cancelled",
            "service_not_found",
            "application_error",
        ] {
            let v = json!({ "type": "response", "id": "r", "ok": false, "error": { "code": code, "message": "safe" } });
            assert!(matches!(
                parse_server_message(&v).unwrap(),
                ServerMessage::Response { outcome: Err(e), .. } if e.code == code
            ));
        }
    }

    #[test]
    fn json_strings_are_not_messages() {
        let hello = json!({ "type": "hello", "version": 8 }).to_string();
        assert!(parse_client_message(&Value::String(hello.clone())).is_err());
        assert!(parse_server_message(&Value::String(hello)).is_err());
    }

    #[test]
    fn encodes_complete_frames_and_enforces_outbound_limits() {
        let hello = ClientMessage::Hello {
            version: PROTOCOL_VERSION,
        };
        let wire = encode_client_message(&hello, DEFAULT_MAX_FRAME_LENGTH).unwrap();
        let mut d = ClientMessageDecoder::default();
        assert_eq!(d.push(&wire).unwrap(), vec![hello.clone()]);
        assert!(encode_client_message(&hello, 8).is_err());
        let server = ServerMessage::Hello {
            server_id: SERVER.into(),
        };
        assert!(encode_server_message(&server, 8).is_err());
    }

    #[test]
    fn decodes_fragmented_and_coalesced_client_messages_at_every_split() {
        let hello = ClientMessage::Hello {
            version: PROTOCOL_VERSION,
        };
        let req = ClientMessage::Request {
            id: "request-1".into(),
            target: RpcTarget::Server {
                server_id: SERVER.into(),
            },
            call: json!({ "serviceId": "pi.session-directory", "member": "list", "args": [] }),
        };
        let mut wire = encode_client_message(&hello, DEFAULT_MAX_FRAME_LENGTH).unwrap();
        wire.extend(encode_client_message(&req, DEFAULT_MAX_FRAME_LENGTH).unwrap());
        for split in 0..=wire.len() {
            let mut d = ClientMessageDecoder::default();
            let mut messages = d.push(&wire[..split]).unwrap();
            messages.extend(d.push(&wire[split..]).unwrap());
            d.end().unwrap();
            assert_eq!(messages, vec![hello.clone(), req.clone()], "split {split}");
        }
    }

    #[test]
    fn decodes_fragmented_server_messages() {
        let hello = ServerMessage::Hello {
            server_id: SERVER.into(),
        };
        let response = ServerMessage::Response {
            id: "request-1".into(),
            outcome: Ok(Some(json!([]))),
        };
        let first = encode_server_message(&hello, DEFAULT_MAX_FRAME_LENGTH).unwrap();
        let second = encode_server_message(&response, DEFAULT_MAX_FRAME_LENGTH).unwrap();
        let mut wire = first.clone();
        wire.extend(&second);
        let split = first.len() + second.len() / 2;
        let mut d = ServerMessageDecoder::default();
        assert_eq!(d.push(&wire[..split]).unwrap(), vec![hello]);
        assert_eq!(d.push(&wire[split..]).unwrap(), vec![response]);
        d.end().unwrap();
    }

    #[test]
    fn a_decoder_that_failed_stays_failed() {
        let cases: Vec<Vec<u8>> = vec![
            encode_frame(&[]).unwrap(),
            encode_frame(&[0xff]).unwrap(),
            encode_frame(
                &cbor::encode(
                    &json!({ "type": "hello", "version": 1, "extra": true }),
                    &Limits::default(),
                )
                .unwrap(),
            )
            .unwrap(),
        ];
        for wire in cases {
            let mut d = ClientMessageDecoder::default();
            assert!(matches!(d.push(&wire), Err(Error::Validation(_))));
            let again = encode_client_message(
                &ClientMessage::Hello { version: 8 },
                DEFAULT_MAX_FRAME_LENGTH,
            )
            .unwrap();
            assert!(d.push(&again).unwrap_err().to_string().contains("failed"));
        }
    }

    #[test]
    fn rejects_truncated_and_oversized_framing() {
        let mut d = ServerMessageDecoder::default();
        assert!(d.push(&[0, 0, 0, 2, 1]).unwrap().is_empty());
        assert!(matches!(d.end(), Err(Error::Validation(_))));
        let mut d = ClientMessageDecoder::new(3);
        assert!(matches!(d.push(&[0, 0, 0, 4]), Err(Error::Validation(_))));
    }
}
