//! Unix-domain socket transport with the access checks the protocol itself lacks.
//!
//! Pi's experimental transport has no peer authentication: reaching the socket is the only
//! credential. So before speaking to an endpoint this module requires that the socket and its
//! directory belong to the current user and are inaccessible to anyone else, that neither is
//! reached through a symlink, and that the kernel reports the peer process as the same user.
//! The `serverId` handshake then confirms *which* logical server answered. A random attachment
//! id or a matching server id is not authentication; these checks are.

use std::fs;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use crate::client::{Client, ClientEvent, ClientOptions};
use crate::error::{Error, Result};
use crate::protocol::is_server_id;

const SOCKET_SUFFIX: &str = ".sock";

fn untrusted(message: impl Into<String>) -> Error {
    Error::Untrusted(message.into())
}

fn effective_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// Require `meta` to be owned by `uid` with no group or other access.
fn require_private(path: &Path, meta: &fs::Metadata, uid: u32, what: &str) -> Result<()> {
    if meta.uid() != uid {
        return Err(untrusted(format!(
            "{what} {} is owned by uid {}, not the current user ({uid})",
            path.display(),
            meta.uid()
        )));
    }
    if meta.mode() & 0o077 != 0 {
        return Err(untrusted(format!(
            "{what} {} is accessible to other users (mode {:o})",
            path.display(),
            meta.mode() & 0o7777
        )));
    }
    Ok(())
}

/// Check the endpoint's filesystem identity before connecting.
pub fn check_endpoint(path: &Path) -> Result<()> {
    let uid = effective_uid();
    let parent = path
        .parent()
        .ok_or_else(|| untrusted("socket path has no directory"))?;
    // `symlink_metadata` never follows: a symlinked directory or socket is not ours to trust.
    let dir = fs::symlink_metadata(parent).map_err(|e| {
        untrusted(format!(
            "cannot inspect directory {}: {e}",
            parent.display()
        ))
    })?;
    if !dir.file_type().is_dir() {
        return Err(untrusted(format!(
            "{} is not a directory (or is a symlink)",
            parent.display()
        )));
    }
    require_private(parent, &dir, uid, "socket directory")?;
    let socket = fs::symlink_metadata(path)
        .map_err(|e| untrusted(format!("cannot inspect socket {}: {e}", path.display())))?;
    if !socket.file_type().is_socket() {
        return Err(untrusted(format!(
            "{} is not a socket (or is a symlink)",
            path.display()
        )));
    }
    require_private(path, &socket, uid, "socket")
}

/// The uid the kernel reports for the process at the other end of `stream`.
pub fn peer_uid(stream: &UnixStream) -> Result<u32> {
    use std::os::fd::AsRawFd;
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `cred` and `len` are valid for the duration of the call and `len` is its size.
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    if rc != 0 {
        return Err(untrusted(format!(
            "cannot read peer credentials: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(cred.uid)
}

/// Connect to a trusted endpoint and complete the handshake for `options.server_id`.
pub fn connect(
    path: &Path,
    options: ClientOptions,
) -> Result<(Client, async_channel::Receiver<ClientEvent>)> {
    check_endpoint(path)?;
    let stream = UnixStream::connect(path)
        .map_err(|e| Error::Disconnected(format!("cannot connect to {}: {e}", path.display())))?;
    let peer = peer_uid(&stream)?;
    if peer != effective_uid() {
        return Err(untrusted(format!(
            "peer process runs as uid {peer}, not the current user"
        )));
    }
    let read = stream
        .try_clone()
        .map_err(|e| Error::Disconnected(e.to_string()))?;
    let shutdown = stream
        .try_clone()
        .map_err(|e| Error::Disconnected(e.to_string()))?;
    Client::establish(
        read,
        stream,
        move || {
            let _ = shutdown.shutdown(std::net::Shutdown::Both);
        },
        options,
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerRoute {
    pub server_id: String,
    pub path: PathBuf,
}

/// What discovery found, and what it refused to talk to and why.
#[derive(Debug, Default)]
pub struct Discovery {
    pub routes: Vec<ServerRoute>,
    /// Entries that look like server sockets but failed an access check. Reported so a caller
    /// can say "your Pi directory is insecure" instead of the misleading "no server running".
    pub untrusted: Vec<(PathBuf, Error)>,
    /// Servers that refused our protocol version. Reported so a caller can say "incompatible"
    /// instead of the misleading "no server running".
    pub incompatible: Vec<(PathBuf, Error)>,
}

/// Find reachable servers by probing `<serverId>.sock` entries in `directory`. Entries that are
/// malformed, not sockets, stale, or that answer as another server are skipped; entries that
/// fail an access check are skipped and reported. Discovery never talks to anything it would
/// not connect to.
pub fn discover(directory: &Path, options: &ClientOptions) -> Result<Discovery> {
    let entries = match fs::read_dir(directory) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Discovery::default()),
        Err(e) => {
            return Err(Error::Disconnected(format!(
                "cannot read {}: {e}",
                directory.display()
            )));
        }
    };
    let mut found = Discovery::default();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(id) = name.strip_suffix(SOCKET_SUFFIX) else {
            continue;
        };
        if !is_server_id(id) {
            continue;
        }
        let path = entry.path();
        let mut probe = options.clone();
        probe.server_id = id.to_owned();
        match connect(&path, probe) {
            Ok((client, _events)) => {
                client.disconnect();
                found.routes.push(ServerRoute {
                    server_id: id.to_owned(),
                    path,
                });
            }
            Err(error @ Error::Untrusted(_)) => found.untrusted.push((path, error)),
            Err(error @ Error::Server { .. }) if matches!(&error, Error::Server { code, .. } if code == "version") =>
            {
                found.incompatible.push((path, error));
            }
            Err(_) => {}
        }
    }
    found.routes.sort_by(|a, b| a.server_id.cmp(&b.server_id));
    Ok(found)
}
