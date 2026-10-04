//! Small platform boundaries. Clipboard and dialogs belong to GPUI; only filesystem
//! locations live here.

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

pub fn db_path(dir: &Path) -> PathBuf {
    dir.join(DB_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

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
