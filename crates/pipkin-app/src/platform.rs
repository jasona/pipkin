//! Small platform boundaries. Clipboard and dialogs belong to GPUI; only filesystem
//! locations and local server identity live here.

use std::io::Read;
use std::path::{Path, PathBuf};

pub const DATA_DIR_ENV: &str = "PIPKIN_DATA";
pub const DB_FILE: &str = "pipkin.sqlite3";

/// Resolve the data directory: explicit CLI value, then `PIPKIN_DATA`, then
/// `$XDG_DATA_HOME/pipkin`, then `~/.local/share/pipkin`.
pub fn data_dir(cli: Option<&Path>) -> PathBuf {
    resolve_data_dir(
        cli,
        std::env::var_os(DATA_DIR_ENV).map(PathBuf::from),
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

pub fn resolve_data_dir(
    cli: Option<&Path>,
    env_override: Option<PathBuf>,
    xdg_data_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> PathBuf {
    let non_empty = |p: &PathBuf| !p.as_os_str().is_empty();
    if let Some(p) = cli {
        return p.to_path_buf();
    }
    if let Some(p) = env_override.filter(non_empty) {
        return p;
    }
    if let Some(p) = xdg_data_home.filter(non_empty) {
        return p.join("pipkin");
    }
    home.filter(non_empty)
        .map(|h| h.join(".local/share/pipkin"))
        .unwrap_or_else(|| PathBuf::from(".pipkin"))
}

/// A canonical UUIDv4 using OS entropy on our current Unix targets, without Linux /proc.
/// Do not substitute timestamps if entropy is unavailable: a unique server identity is required.
pub fn fresh_server_id() -> std::io::Result<String> {
    let mut bytes = [0_u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

pub fn db_path(dir: &Path) -> PathBuf {
    dir.join(DB_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_ids_are_distinct_canonical_protocol_uuid_v4() {
        let first = fresh_server_id().unwrap();
        let second = fresh_server_id().unwrap();
        assert!(pi_client::protocol::is_server_id(&first));
        assert!(pi_client::protocol::is_server_id(&second));
        assert_ne!(first, second);
    }

    #[test]
    fn precedence_is_cli_env_xdg_home() {
        let p = |s: &str| Some(PathBuf::from(s));
        assert_eq!(
            resolve_data_dir(Some(Path::new("/cli")), p("/env"), p("/xdg"), p("/home/u")),
            PathBuf::from("/cli")
        );
        assert_eq!(
            resolve_data_dir(None, p("/env"), p("/xdg"), p("/home/u")),
            PathBuf::from("/env")
        );
        assert_eq!(
            resolve_data_dir(None, None, p("/xdg"), p("/home/u")),
            PathBuf::from("/xdg/pipkin")
        );
        assert_eq!(
            resolve_data_dir(None, p(""), None, p("/home/u")),
            PathBuf::from("/home/u/.local/share/pipkin")
        );
    }
}
