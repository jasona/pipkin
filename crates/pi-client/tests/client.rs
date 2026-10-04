//! The client against the mock server over real socket pairs and the real wire.

use std::time::Duration;

use pi_client::Error;
use pi_client::chord::{Change, Mode, ServiceCall};
use pi_client::client::{ClientEvent, ClientOptions};
use pi_client::delta::{Op, Seg};
use pi_client::protocol::{RpcTarget, SessionTarget};
use pi_client::testing::{MockPi, SERVER_ID, install_session_management};
use serde_json::{Value, json};

const T: Duration = Duration::from_secs(5);

fn server() -> RpcTarget {
    RpcTarget::Server {
        server_id: SERVER_ID.into(),
    }
}

fn key(s: &str) -> Seg {
    Seg::Key(s.into())
}

fn set(path: &str, value: Value) -> Op {
    Op::Set(vec![key(path)], value)
}

fn next_event(rx: &async_channel::Receiver<ClientEvent>) -> ClientEvent {
    rx.recv_blocking().expect("an event")
}

/// Wait for the first event matching `f`, skipping others.
fn wait_for(
    rx: &async_channel::Receiver<ClientEvent>,
    f: impl Fn(&ClientEvent) -> bool,
) -> ClientEvent {
    loop {
        let event = next_event(rx);
        if f(&event) {
            return event;
        }
    }
}

fn pi_with_directory() -> MockPi {
    let pi = MockPi::default();
    pi.add_service(
        "pi.session-directory",
        &[],
        Some(json!({ "revision": 1, "sessions": [] })),
    );
    install_session_management(&pi);
    pi
}

#[test]
fn handshake_succeeds_against_the_expected_server() {
    let pi = pi_with_directory();
    let mc = pi.connect().unwrap();
    assert!(mc.client.is_connected());
    assert_eq!(mc.client.server_id(), SERVER_ID);
}

#[test]
fn a_different_server_id_is_rejected() {
    let pi = MockPi::new("00000000-0000-4000-8000-0000000000aa");
    // The client expects the default id; the endpoint answers as another server.
    let err = pi.connect().err().expect("must refuse");
    assert!(err.to_string().contains("does not match"), "{err}");
}

#[test]
fn an_invalid_expected_server_id_is_refused_up_front() {
    let pi = MockPi::default();
    assert!(pi.connect_with(ClientOptions::new("not-a-uuid")).is_err());
}

#[test]
fn catalogue_lists_services() {
    let pi = pi_with_directory();
    let mc = pi.connect().unwrap();
    let catalogue = mc.client.catalogue(&server(), T).unwrap();
    let ids: Vec<_> = catalogue.iter().map(|e| e.service_id.as_str()).collect();
    assert_eq!(ids, ["pi.session-directory", "pi.session-management"]);
}

#[test]
fn subscription_hydrates_then_follows_published_updates() {
    let pi = pi_with_directory();
    let mc = pi.connect().unwrap();
    let sub = mc
        .client
        .subscribe(&server(), "pi.session-directory", Mode::Singleton, T)
        .unwrap();
    assert_eq!(
        sub.read(|r| r.state("state").cloned()).unwrap(),
        Some(json!({ "revision": 1, "sessions": [] }))
    );

    pi.publish(
        "pi.session-directory",
        vec![
            set("revision", json!(2)),
            Op::Splice {
                path: vec![key("sessions")],
                index: 0,
                remove: 0,
                items: vec![json!({ "sessionId": "s1", "createdAt": 5 })],
            },
        ],
    );
    let event = wait_for(&mc.events, |e| {
        matches!(e, ClientEvent::SubscriptionChanged { .. })
    });
    assert_eq!(
        event,
        ClientEvent::SubscriptionChanged {
            subscription: sub.id().into(),
            change: Change::State {
                instance: None,
                member: "state".into()
            }
        }
    );
    let state = sub.read(|r| r.state("state").cloned()).unwrap().unwrap();
    assert_eq!(state["revision"], 2);
    assert_eq!(state["sessions"][0]["sessionId"], "s1");
}

#[test]
fn many_updates_converge_through_the_path_dictionary() {
    let pi = pi_with_directory();
    let mc = pi.connect().unwrap();
    let sub = mc
        .client
        .subscribe(&server(), "pi.session-directory", Mode::Singleton, T)
        .unwrap();
    for i in 0..200u64 {
        pi.publish("pi.session-directory", vec![set("revision", json!(i + 2))]);
    }
    let mut seen = 0;
    while seen < 200 {
        if matches!(
            next_event(&mc.events),
            ClientEvent::SubscriptionChanged { .. }
        ) {
            seen += 1;
        }
    }
    assert_eq!(
        sub.read(|r| r.state("state").unwrap()["revision"].clone())
            .unwrap(),
        201
    );
    assert_eq!(sub.read(|r| r.sequence(&None, "state")).unwrap(), Some(200));
}

#[test]
fn void_and_valued_method_calls_round_trip() {
    let pi = pi_with_directory();
    pi.set_handler("pi.session-management", "create", |_, _, call| {
        Ok(Some(json!({ "echo": call.args.clone() })))
    });
    let mc = pi.connect().unwrap();
    let result = mc
        .client
        .request(
            &server(),
            &ServiceCall::new(
                "pi.session-management",
                "create",
                vec![json!({ "id": "x" })],
            ),
        )
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(result, Some(json!({ "echo": [{ "id": "x" }] })));
    // A void response carries no result.
    let void = mc
        .client
        .request(
            &server(),
            &ServiceCall::new("pi.session-management", "detach", vec![]),
        )
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(void, None);
}

#[test]
fn server_errors_surface_with_their_code() {
    let pi = pi_with_directory();
    let mc = pi.connect().unwrap();
    let err = mc
        .client
        .request(
            &server(),
            &ServiceCall::new("pi.session-management", "nope", vec![]),
        )
        .unwrap()
        .wait()
        .unwrap_err();
    assert!(
        matches!(err, Error::Server { ref code, .. } if code == "method_not_found"),
        "{err}"
    );
    // The connection survives an application error.
    assert!(mc.client.is_connected());
}

#[test]
fn attach_publishes_a_route_and_switching_retires_old_session_subscriptions() {
    let pi = pi_with_directory();
    pi.add_service("pi.models", &["select"], Some(json!({ "n": 0 })));
    let mc = pi.connect().unwrap();
    let attach = |id: &str| {
        mc.client
            .request(
                &server(),
                &ServiceCall::new("pi.session-management", "attach", vec![json!(id)]),
            )
            .unwrap()
            .wait()
            .unwrap();
    };

    attach("session-a");
    let ClientEvent::Attachment(Some(first)) =
        wait_for(&mc.events, |e| matches!(e, ClientEvent::Attachment(_)))
    else {
        panic!()
    };
    assert_eq!(first.session_id, "session-a");
    assert_eq!(mc.client.attachment(), Some(first.clone()));

    let target_a = first.rpc();
    let sub_a = mc
        .client
        .subscribe(&target_a, "pi.models", Mode::Singleton, T)
        .unwrap();
    assert!(!sub_a.is_retired());

    attach("session-b");
    let ClientEvent::Attachment(Some(second)) =
        wait_for(&mc.events, |e| matches!(e, ClientEvent::Attachment(_)))
    else {
        panic!()
    };
    assert_eq!(second.session_id, "session-b");
    assert_ne!(first.attachment_id, second.attachment_id);

    // The old subscription is retired and unreadable.
    wait_for(&mc.events, |e| {
        matches!(e, ClientEvent::SubscriptionRetired { .. })
    });
    assert!(sub_a.is_retired());
    assert!(sub_a.read(|_| ()).is_none());

    // A delayed update for it, still in flight from the old attachment, is dropped.
    let old_id = sub_a.id().to_string();
    mc.conn
        .send_update(&old_id, json!({ "type": "unavailable" }));
    pi.publish("pi.models", vec![set("n", json!(1))]);
    // Fence: a round trip guarantees the injected update has been read and processed.
    mc.client
        .request(
            &server(),
            &ServiceCall::new("pi.session-management", "detach", vec![]),
        )
        .unwrap()
        .wait()
        .unwrap();
    assert!(mc.client.stale_updates_dropped() >= 1);
    assert!(mc.client.is_connected());

    // Calls to the old attachment are refused locally and never reach the server.
    let before = pi.requests().len();
    let err = mc
        .client
        .request(&target_a, &ServiceCall::new("pi.models", "select", vec![]))
        .err()
        .expect("stale route refused");
    assert!(err.to_string().contains("stale route"), "{err}");
    assert_eq!(pi.requests().len(), before);
}

#[test]
fn detach_clears_the_attachment_and_retires_its_subscriptions() {
    let pi = pi_with_directory();
    pi.add_service("pi.models", &[], Some(json!({ "n": 0 })));
    let mc = pi.connect().unwrap();
    mc.client
        .request(
            &server(),
            &ServiceCall::new("pi.session-management", "attach", vec![json!("s")]),
        )
        .unwrap()
        .wait()
        .unwrap();
    wait_for(&mc.events, |e| {
        matches!(e, ClientEvent::Attachment(Some(_)))
    });
    let target = mc.client.attachment().unwrap().rpc();
    let sub = mc
        .client
        .subscribe(&target, "pi.models", Mode::Singleton, T)
        .unwrap();
    mc.client
        .request(
            &server(),
            &ServiceCall::new("pi.session-management", "detach", vec![]),
        )
        .unwrap()
        .wait()
        .unwrap();
    wait_for(&mc.events, |e| matches!(e, ClientEvent::Attachment(None)));
    wait_for(&mc.events, |e| {
        matches!(e, ClientEvent::SubscriptionRetired { .. })
    });
    assert!(sub.is_retired());
    assert_eq!(mc.client.attachment(), None);
}

#[test]
fn a_sequence_gap_fails_only_that_subscription() {
    let pi = pi_with_directory();
    let mc = pi.connect().unwrap();
    let sub = mc
        .client
        .subscribe(&server(), "pi.session-directory", Mode::Singleton, T)
        .unwrap();
    // Skip sequence 1 by injecting sequence 2 directly.
    let id = sub.id().to_string();
    mc.conn.send_update(&id, json!({ "type": "state", "member": "state", "sequence": 2, "ops": [["s", ["revision"], 9]] }));
    let event = wait_for(&mc.events, |e| {
        matches!(e, ClientEvent::SubscriptionFailed { .. })
    });
    assert!(matches!(
        event,
        ClientEvent::SubscriptionFailed {
            error: Error::SequenceGap {
                expected: 1,
                got: 2
            },
            ..
        }
    ));
    assert!(sub.read(|_| ()).is_none());
    assert!(matches!(sub.error(), Some(Error::SequenceGap { .. })));
    // The connection and other traffic are unaffected, and a fresh subscription recovers.
    assert!(mc.client.is_connected());
    let fresh = mc
        .client
        .subscribe(&server(), "pi.session-directory", Mode::Singleton, T)
        .unwrap();
    assert_eq!(
        fresh
            .read(|r| r.state("state").unwrap()["revision"].clone())
            .unwrap(),
        1
    );
}

#[test]
fn an_undecodable_op_stream_fails_the_connection() {
    let pi = pi_with_directory();
    let mc = pi.connect().unwrap();
    let sub = mc
        .client
        .subscribe(&server(), "pi.session-directory", Mode::Singleton, T)
        .unwrap();
    let id = sub.id().to_string();
    // A path id that was never defined is a protocol violation, not a recoverable gap.
    mc.conn.send_update(
        &id,
        json!({ "type": "state", "member": "state", "sequence": 1, "ops": [["s", 7, 1]] }),
    );
    let event = wait_for(&mc.events, |e| matches!(e, ClientEvent::Disconnected(_)));
    assert!(
        matches!(event, ClientEvent::Disconnected(Error::Delta(_))),
        "{event:?}"
    );
    assert!(!mc.client.is_connected());
    assert!(sub.is_retired());
}

#[test]
fn updates_that_race_ahead_of_the_snapshot_are_buffered_in_order() {
    let pi = pi_with_directory();
    let mc = pi.connect().unwrap();
    // Subscribe and publish concurrently: whatever the interleaving, the replica must end at
    // the server's final revision with no gap.
    let publisher = {
        let pi = pi.clone();
        std::thread::spawn(move || {
            for i in 0..50u64 {
                pi.publish("pi.session-directory", vec![set("revision", json!(i + 2))]);
            }
        })
    };
    let sub = mc
        .client
        .subscribe(&server(), "pi.session-directory", Mode::Singleton, T)
        .unwrap();
    publisher.join().unwrap();
    let want = pi.state_of("pi.session-directory").unwrap()["revision"].clone();
    // Drain until the replica catches up.
    for _ in 0..2000 {
        if sub
            .read(|r| r.state("state").unwrap()["revision"] == want)
            .unwrap_or(false)
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("replica never converged: {:?}", sub.error());
}

#[test]
fn disconnect_rejects_pending_requests_and_retires_subscriptions_without_replay() {
    let pi = pi_with_directory();
    pi.set_handler("pi.session-management", "create", |_, conn, _| {
        conn.close(); // the server dies mid-call, before any response
        Ok(None)
    });
    let mc = pi.connect().unwrap();
    let sub = mc
        .client
        .subscribe(&server(), "pi.session-directory", Mode::Singleton, T)
        .unwrap();
    let err = mc
        .client
        .request(
            &server(),
            &ServiceCall::new("pi.session-management", "create", vec![]),
        )
        .unwrap()
        .wait()
        .unwrap_err();
    assert!(matches!(err, Error::Disconnected(_)), "{err}");
    wait_for(&mc.events, |e| matches!(e, ClientEvent::Disconnected(_)));
    assert!(!mc.client.is_connected());
    assert!(sub.is_retired());
    // Nothing was resent: the server saw the call exactly once.
    assert_eq!(pi.requests().len(), 1);
    // New requests fail fast.
    assert!(
        mc.client
            .request(&server(), &ServiceCall::new("x", "y", vec![]))
            .is_err()
    );
}

#[test]
fn a_response_for_an_unknown_request_is_a_protocol_violation() {
    let pi = pi_with_directory();
    let mc = pi.connect().unwrap();
    let frame = pi_client::protocol::encode_server_message(
        &pi_client::protocol::ServerMessage::Response {
            id: "request-999".into(),
            outcome: Ok(None),
        },
        1 << 20,
    )
    .unwrap();
    mc.conn.send_raw(&frame);
    let event = wait_for(&mc.events, |e| matches!(e, ClientEvent::Disconnected(_)));
    assert!(
        matches!(event, ClientEvent::Disconnected(Error::Validation(_))),
        "{event:?}"
    );
}

#[test]
fn malformed_and_oversized_frames_fail_the_connection() {
    let pi = pi_with_directory();
    let mc = pi.connect().unwrap();
    mc.conn.send_raw(&[0, 0, 0, 1, 0xff]); // valid length, invalid CBOR
    wait_for(&mc.events, |e| matches!(e, ClientEvent::Disconnected(_)));

    let mc = pi
        .connect_with(ClientOptions {
            max_frame_length: 100,
            ..ClientOptions::new(SERVER_ID)
        })
        .unwrap();
    mc.conn.send_raw(&[0, 0, 1, 0]); // declares 256 bytes against a 100-byte limit
    let event = wait_for(&mc.events, |e| matches!(e, ClientEvent::Disconnected(_)));
    assert!(event != ClientEvent::Attachment(None));
}

#[test]
fn fragmented_delivery_is_reassembled() {
    let pi = pi_with_directory();
    let mc = pi.connect().unwrap();
    let sub = mc
        .client
        .subscribe(&server(), "pi.session-directory", Mode::Singleton, T)
        .unwrap();
    let id = sub.id().to_string();
    let update = json!({ "type": "state", "member": "state", "sequence": 1, "ops": [["s", ["revision"], 42]] });
    let frame = pi_client::protocol::encode_server_message(
        &pi_client::protocol::ServerMessage::ServiceUpdate {
            subscription_id: id,
            update,
        },
        1 << 20,
    )
    .unwrap();
    for byte in frame {
        mc.conn.send_raw(&[byte]);
    }
    wait_for(&mc.events, |e| {
        matches!(e, ClientEvent::SubscriptionChanged { .. })
    });
    assert_eq!(
        sub.read(|r| r.state("state").unwrap()["revision"].clone())
            .unwrap(),
        42
    );
}

#[test]
fn cancel_withdraws_the_wait_and_a_late_response_is_not_a_violation() {
    let pi = pi_with_directory();
    pi.set_handler("pi.session-management", "create", |_, _, _| {
        std::thread::sleep(Duration::from_millis(150));
        Ok(None)
    });
    let mc = pi.connect().unwrap();
    let pending = mc
        .client
        .request(
            &server(),
            &ServiceCall::new("pi.session-management", "create", vec![]),
        )
        .unwrap();
    assert_eq!(
        pending.wait_timeout(Duration::from_millis(10)).unwrap_err(),
        Error::Timeout
    );
    pending.cancel();
    // The server still answers; the late response must not tear the connection down.
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        mc.client.is_connected(),
        "a late response to a cancelled request is expected"
    );
}

#[test]
fn explicit_disconnect_is_final() {
    let pi = pi_with_directory();
    let mc = pi.connect().unwrap();
    mc.client.disconnect();
    wait_for(&mc.events, |e| matches!(e, ClientEvent::Disconnected(_)));
    assert!(!mc.client.is_connected());
    assert!(mc.client.catalogue(&server(), T).is_err());
}

#[test]
fn subscription_to_an_unknown_service_is_an_application_error() {
    let pi = pi_with_directory();
    let mc = pi.connect().unwrap();
    let err = mc
        .client
        .subscribe(&server(), "pi.nope", Mode::Singleton, T)
        .err()
        .unwrap();
    assert!(matches!(err, Error::Server { ref code, .. } if code == "service_not_found"));
    assert!(mc.client.is_connected());
}

#[test]
fn session_target_helper_builds_the_route() {
    let t = SessionTarget {
        server_id: SERVER_ID.into(),
        session_id: "s".into(),
        attachment_id: "a".into(),
    };
    assert!(matches!(t.rpc(), RpcTarget::Session { .. }));
}
