#![cfg(unix)] // a fake herdr server on a Unix socket
//! Client tests against the in-test fake herdr (replaces the Swift FakeHerdr / store-timing use).
mod common;
use common::*;
use herdr_core::{HerdrClient, HerdrError, HerdrSession};
use std::time::{Duration, Instant};

fn session_for(path: &std::path::Path) -> HerdrSession {
    HerdrSession { name: Some("fake".into()), socket_path: path.to_string_lossy().into_owned() }
}

fn start() -> (FakeHerdr, HerdrClient<herdr_core::UnixConnector>) {
    let dir = temp_dir("c");
    let fake = FakeHerdr::new(dir.join("s.sock"));
    fake.start();
    let client = HerdrClient::new(&session_for(&fake.path));
    (fake, client)
}

#[test]
fn snapshot_roundtrip_and_request_ids() {
    let (fake, client) = start();
    let ws = client.snapshot().unwrap();
    assert!(ws.len() == 1 && ws[0].label == "v0" && ws[0].tabs[0].panes[0].terminal_id == "term_1");
    fake.set_label("v1");
    assert_eq!(client.snapshot().unwrap()[0].label, "v1");
    assert_eq!(fake.snapshot_count(), 2);
    assert_eq!(fake.methods_seen(), ["session.snapshot", "session.snapshot"]);
    // The id echoes back as `<prefix>:<n>` with n counting up from 1.
    let line = client.raw_request("session.snapshot", serde_json::json!({})).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&line).unwrap();
    assert_eq!(v["id"], "hk:3");
    fake.kill();
}

#[test]
fn server_error_maps_to_error() {
    let (fake, client) = start();
    match client.raw_request("no.such.method", serde_json::json!({})) {
        Err(HerdrError::Server { code, message }) => assert!(code == "unknown" && message == "fake"),
        other => panic!("expected server error, got {other:?}"),
    }
    fake.kill();
}

#[test]
fn garbage_response_is_protocol_error() {
    let (fake, client) = start();
    match client.raw_request("test.garbage", serde_json::json!({})) {
        Err(HerdrError::Protocol(_)) => {}
        other => panic!("expected protocol error, got {other:?}"),
    }
    fake.kill();
}

#[test]
fn connect_failure_is_socket_error() {
    let dir = temp_dir("n");
    let client = HerdrClient::new(&session_for(&dir.join("absent.sock")));
    assert!(matches!(client.snapshot(), Err(HerdrError::Socket(m)) if m.contains("connect")));
}

#[test]
fn stuck_server_times_out() {
    let dir = temp_dir("t");
    let fake = FakeHerdr::new(dir.join("s.sock"));
    fake.start();
    fake.set_stall_ms(3000);
    let client = HerdrClient::new(&session_for(&fake.path)).with_timeout(Duration::from_millis(300));
    let t = Instant::now();
    let r = client.snapshot();
    // As Swift's LineSocket (`.timeout` on EAGAIN): the request may still take effect, so it is not a plain failure.
    assert!(matches!(r, Err(HerdrError::Timeout(_))), "timeout surfaces as a timeout error: {r:?}");
    assert!(t.elapsed() < Duration::from_millis(2000), "gave up near the timeout, not after the stall");
    fake.kill();
}

#[test]
fn event_stream_push_latency_and_socket_loss() {
    let (fake, client) = start();
    let mut ev = client.events().unwrap();
    assert!(ev.next().unwrap().is_ok(), "subscription ack yields once");
    let t = Instant::now();
    fake.push_event();
    assert!(ev.next().unwrap().is_ok(), "pushed event arrives");
    eprintln!("event latency: {:?}", t.elapsed());
    assert!(t.elapsed() < Duration::from_millis(100), "event latency {:?}", t.elapsed());
    fake.kill();
    // After the socket is lost the stream ends with an error, then finishes.
    match ev.next() {
        Some(Err(HerdrError::Closed)) | Some(Err(HerdrError::Socket(_))) => {}
        other => panic!("expected closed, got {other:?}"),
    }
    assert!(ev.next().is_none());
    // The socket is gone: a snapshot fails now, and works again after the server comes back.
    assert!(client.snapshot().is_err());
    fake.set_label("r1");
    fake.start();
    assert_eq!(client.snapshot().unwrap()[0].label, "r1");
    fake.kill();
}
