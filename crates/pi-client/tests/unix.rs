//! The Unix transport's trust checks, against real sockets on disk.

use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};

use pi_client::Error;
use pi_client::client::ClientOptions;
use pi_client::testing::{MockPi, SERVER_ID, install_session_management};
use pi_client::unix::{check_endpoint, connect, discover, peer_uid};

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    socket: PathBuf,
}

fn chmod(path: &Path, mode: u32) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// A private directory holding a listening socket served by the mock.
fn fixture(pi: &MockPi, server_id: &str) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("server");
    std::fs::create_dir(&root).unwrap();
    chmod(&root, 0o700);
    let socket = root.join(format!("{server_id}.sock"));
    let listener = UnixListener::bind(&socket).unwrap();
    chmod(&socket, 0o600);
    let pi = pi.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            pi.accept(stream);
        }
    });
    Fixture {
        _dir: dir,
        root,
        socket,
    }
}

fn pi() -> MockPi {
    let pi = MockPi::default();
    install_session_management(&pi);
    pi
}

#[test]
fn connects_to_a_private_endpoint_and_handshakes() {
    let fx = fixture(&pi(), SERVER_ID);
    check_endpoint(&fx.socket).unwrap();
    let (client, _events) = connect(&fx.socket, ClientOptions::new(SERVER_ID)).unwrap();
    assert!(client.is_connected());
}

#[test]
fn refuses_a_directory_open_to_other_users() {
    let fx = fixture(&pi(), SERVER_ID);
    chmod(&fx.root, 0o755);
    let err = connect(&fx.socket, ClientOptions::new(SERVER_ID))
        .err()
        .unwrap();
    assert!(
        matches!(err, Error::Untrusted(ref m) if m.contains("directory")),
        "{err}"
    );
    // Group access alone is also refused.
    chmod(&fx.root, 0o770);
    assert!(matches!(
        check_endpoint(&fx.socket),
        Err(Error::Untrusted(_))
    ));
}

#[test]
fn refuses_a_socket_open_to_other_users() {
    let fx = fixture(&pi(), SERVER_ID);
    chmod(&fx.socket, 0o666);
    let err = connect(&fx.socket, ClientOptions::new(SERVER_ID))
        .err()
        .unwrap();
    assert!(
        matches!(err, Error::Untrusted(ref m) if m.contains("socket")),
        "{err}"
    );
}

#[test]
fn refuses_a_symlinked_socket_and_a_regular_file() {
    let fx = fixture(&pi(), SERVER_ID);
    let link = fx.root.join("link.sock");
    symlink(&fx.socket, &link).unwrap();
    assert!(matches!(check_endpoint(&link), Err(Error::Untrusted(_))));

    let file = fx.root.join("plain.sock");
    std::fs::write(&file, b"not a socket").unwrap();
    chmod(&file, 0o600);
    assert!(matches!(check_endpoint(&file), Err(Error::Untrusted(_))));
}

#[test]
fn refuses_a_symlinked_directory() {
    let fx = fixture(&pi(), SERVER_ID);
    let alias = fx._dir.path().join("alias");
    symlink(&fx.root, &alias).unwrap();
    let through = alias.join(format!("{SERVER_ID}.sock"));
    assert!(matches!(check_endpoint(&through), Err(Error::Untrusted(_))));
}

#[test]
fn a_missing_endpoint_is_untrusted_not_a_panic() {
    let fx = fixture(&pi(), SERVER_ID);
    assert!(check_endpoint(&fx.root.join("missing.sock")).is_err());
}

#[test]
fn a_wrong_logical_server_is_rejected_even_on_a_trusted_socket() {
    // The path says one server, the process answers as another.
    let other = "00000000-0000-4000-8000-0000000000bb";
    let fx = fixture(&pi(), other);
    let err = connect(&fx.socket, ClientOptions::new(other))
        .err()
        .unwrap();
    assert!(err.to_string().contains("does not match"), "{err}");
}

#[test]
fn peer_credentials_report_the_current_user() {
    let (a, _b) = std::os::unix::net::UnixStream::pair().unwrap();
    assert_eq!(peer_uid(&a).unwrap(), unsafe { libc::geteuid() });
}

#[test]
fn discovery_finds_only_live_trusted_servers() {
    let pi_a = pi();
    let fx = fixture(&pi_a, SERVER_ID);
    // Noise discovery must ignore: wrong suffix, non-UUID name, stale socket, regular file.
    std::fs::write(fx.root.join("default-server-id"), SERVER_ID).unwrap();
    std::fs::write(fx.root.join("notes.sock"), b"x").unwrap();
    let stale = fx.root.join("00000000-0000-4000-8000-0000000000cc.sock");
    drop(UnixListener::bind(&stale).unwrap()); // bound then closed: nothing answers
    chmod(&stale, 0o600); // a stale socket Pi would have left: private, so merely ignored
    let found = discover(&fx.root, &ClientOptions::new(SERVER_ID)).unwrap();
    assert_eq!(found.routes.len(), 1);
    assert_eq!(found.routes[0].server_id, SERVER_ID);
    assert_eq!(found.routes[0].path, fx.socket);
    assert!(found.untrusted.is_empty());
}

#[test]
fn discovery_of_a_missing_directory_is_empty() {
    let fx = fixture(&pi(), SERVER_ID);
    let found = discover(&fx.root.join("nope"), &ClientOptions::new(SERVER_ID)).unwrap();
    assert!(found.routes.is_empty() && found.untrusted.is_empty());
}

#[test]
fn discovery_reports_untrusted_sockets_instead_of_hiding_them() {
    let fx = fixture(&pi(), SERVER_ID);
    chmod(&fx.root, 0o755);
    let found = discover(&fx.root, &ClientOptions::new(SERVER_ID)).unwrap();
    assert!(found.routes.is_empty());
    assert_eq!(found.untrusted.len(), 1);
    assert_eq!(found.untrusted[0].0, fx.socket);
    assert!(matches!(
        found.untrusted[0].1,
        pi_client::Error::Untrusted(_)
    ));
}
