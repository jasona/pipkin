//! Wire streams produced by Pi's REAL server-side code (`scripts/capture-pi-views.ts`: the
//! `Transcript` provider behind Chord's remote endpoint, one TypeScript state encoder per
//! subscription) replayed through this crate's decoder and replica. Convergence to the exact
//! state Pi ends at is the conformance claim: it exercises real path interning, splices, sets
//! and deletes as the engine emits them, not as this crate's own encoder would.

use pi_client::cbor::{self, Limits};
use pi_client::chord::{Replica, StateDecoder, parse_wire_snapshot, parse_wire_update};
use serde_json::Value;

fn stream(name: &str) -> Value {
    let raw = match name {
        "text" => include_str!("../../../fixtures/pi/stream-text.json"),
        "long" => include_str!("../../../fixtures/pi/stream-long.json"),
        "tool" => include_str!("../../../fixtures/pi/stream-tool.json"),
        "multi" => include_str!("../../../fixtures/pi/stream-multi.json"),
        "abort" => include_str!("../../../fixtures/pi/stream-abort.json"),
        other => panic!("unknown stream {other}"),
    };
    serde_json::from_str(raw).expect("fixture is JSON")
}

const ALL: [&str; 5] = ["text", "long", "tool", "multi", "abort"];

fn replay(name: &str) -> (Replica, Value) {
    let data = stream(name);
    let wire =
        parse_wire_snapshot(&data["snapshot"]).unwrap_or_else(|e| panic!("{name}: snapshot: {e}"));
    let mut decoder = StateDecoder::new();
    let decoded = decoder
        .decode_snapshot(&wire)
        .unwrap_or_else(|e| panic!("{name}: decode snapshot: {e}"));
    let mut replica = Replica::hydrate(decoded).unwrap_or_else(|e| panic!("{name}: hydrate: {e}"));
    let base = replica.sequence(&None, "state").expect("a state member");
    for (i, update) in data["updates"].as_array().unwrap().iter().enumerate() {
        let wire = parse_wire_update(update).unwrap_or_else(|e| panic!("{name}: update {i}: {e}"));
        let decoded = decoder
            .decode_update(&wire)
            .unwrap_or_else(|e| panic!("{name}: decode {i}: {e}"));
        replica
            .apply(decoded)
            .unwrap_or_else(|e| panic!("{name}: apply {i}: {e}"));
        assert_eq!(
            replica.sequence(&None, "state"),
            Some(base + 1 + i as u64),
            "{name}: update {i} is contiguous"
        );
    }
    (replica, data["final"].clone())
}

#[test]
fn every_real_stream_converges_to_the_state_pi_ended_at() {
    for name in ALL {
        let (replica, final_state) = replay(name);
        assert!(replica.is_ready(), "{name}");
        assert_eq!(replica.state("state"), Some(&final_state), "{name}");
    }
}

#[test]
fn the_streams_cover_real_path_interning_and_each_op_kind_pi_emits() {
    let mut verbs = std::collections::BTreeSet::new();
    let mut defines = 0;
    for name in ALL {
        for update in stream(name)["updates"].as_array().unwrap() {
            for op in update["ops"].as_array().into_iter().flatten() {
                let verb = op[0].as_str().unwrap().to_owned();
                if verb == "#" {
                    defines += 1;
                }
                verbs.insert(verb);
            }
        }
    }
    assert!(defines > 0, "real dictionaries were exercised");
    for verb in ["s", "p", "d", "#"] {
        assert!(
            verbs.contains(verb),
            "no real `{verb}` op in the captured streams: {verbs:?}"
        );
    }
}

#[test]
fn real_wire_values_survive_the_strict_cbor_subset() {
    // Everything the server sends crosses framed CBOR, so real values must encode and decode
    // unchanged under this crate's strict codec (safe integers, finite floats, string keys).
    let limits = Limits::default();
    for name in ALL {
        let data = stream(name);
        for value in std::iter::once(&data["snapshot"]).chain(data["updates"].as_array().unwrap()) {
            let bytes =
                cbor::encode(value, &limits).unwrap_or_else(|e| panic!("{name}: encode: {e}"));
            assert_eq!(&cbor::decode(&bytes, &limits).unwrap(), value, "{name}");
        }
    }
}

#[test]
fn a_real_stream_with_a_dropped_update_is_a_gap_not_silent_corruption() {
    let data = stream("tool");
    let wire = parse_wire_snapshot(&data["snapshot"]).unwrap();
    let mut decoder = StateDecoder::new();
    let mut replica = Replica::hydrate(decoder.decode_snapshot(&wire).unwrap()).unwrap();
    let updates = data["updates"].as_array().unwrap();
    assert!(updates.len() >= 3);
    // Deliver the first update, skip the second, then deliver the third.
    let first = decoder
        .decode_update(&parse_wire_update(&updates[0]).unwrap())
        .unwrap();
    replica.apply(first).unwrap();
    // The decoder's path dictionary is per-stream, so skipping a batch may already fail to
    // decode; either failure is the protocol detecting the loss, which is the point.
    let skipped = parse_wire_update(&updates[2])
        .map_err(|e| e.to_string())
        .and_then(|w| decoder.decode_update(&w).map_err(|e| e.to_string()))
        .and_then(|u| replica.apply(u).map_err(|e| e.to_string()));
    assert!(skipped.is_err(), "a missing update must be detected");
}
