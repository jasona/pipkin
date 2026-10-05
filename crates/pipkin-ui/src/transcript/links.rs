//! Resolve document links in agent replies against the open project before handing them to GPUI.
//! Linux's URI launcher requires a scheme; `docs/foo.md` is not a URI by itself.

use std::path::{Path, PathBuf};

fn decode_path(text: &str) -> Result<String, String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut it = text.as_bytes().iter().copied();
    while let Some(b) = it.next() {
        if b == b'%' {
            let pair = [it.next(), it.next()];
            let (Some(a), Some(b)) = (pair[0], pair[1]) else {
                return Err("Invalid percent escape in document link".into());
            };
            let hex = |b: u8| (b as char).to_digit(16).map(|n| n as u8);
            let (Some(a), Some(b)) = (hex(a), hex(b)) else {
                return Err("Invalid percent escape in document link".into());
            };
            bytes.push(a * 16 + b);
        } else {
            bytes.push(b);
        }
    }
    String::from_utf8(bytes).map_err(|_| "Document link is not UTF-8".into())
}

fn file_uri(path: &Path) -> Result<String, String> {
    let raw = path.to_str().ok_or("Document path is not UTF-8")?;
    let mut url = String::from("file://");
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            url.push(char::from(byte));
        } else {
            url.push_str(&format!("%{byte:02X}"));
        }
    }
    Ok(url)
}

/// Remote links go to the browser; project-local documents go to the default document handler.
/// Reject unsupported schemes and paths escaping the project, even through symlinks.
pub fn target(link: &str, project: Option<&str>) -> Result<String, String> {
    let link = link.trim();
    if link.starts_with("https://") || link.starts_with("http://") || link.starts_with("mailto:") {
        return Ok(link.to_owned());
    }
    let path = if let Some(file) = link.strip_prefix("file://") {
        // Only local file URIs; never interpret a remote authority as a path.
        if !file.starts_with('/') {
            return Err("Only local document links can be opened".into());
        }
        file
    } else {
        if link.contains(':')
            && link
                .split(':')
                .next()
                .is_some_and(|prefix| !prefix.contains('/'))
        {
            return Err("Unsupported document link scheme".into());
        }
        link
    };
    let path = decode_path(path.split('#').next().unwrap_or(""))?;
    if path.is_empty() {
        return Err("This link does not name a document".into());
    }
    let root = project.ok_or("Open a project to follow document links")?;
    let root = Path::new(root)
        .canonicalize()
        .map_err(|_| "Project folder is unavailable".to_string())?;
    let candidate = PathBuf::from(&path);
    let candidate = if candidate.is_absolute() {
        candidate
    } else {
        root.join(candidate)
    };
    let real = candidate
        .canonicalize()
        .map_err(|_| format!("Document not found: {path}"))?;
    if !real.starts_with(&root) || !real.is_file() {
        return Err("Document link must point to a file inside this project".into());
    }
    file_uri(&real)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_links_get_file_uris_and_external_links_stay_as_they_are() {
        let dir = std::env::temp_dir().join(format!("pipkin-link-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("docs/My notes.md"), "hello").unwrap();
        let project = dir.to_str().unwrap();
        let expected = format!("file://{}/docs/My%20notes.md", dir.display());
        assert_eq!(
            target("docs/My%20notes.md#intro", Some(project)).unwrap(),
            expected
        );
        assert_eq!(
            target("https://pipkinai.com", Some(project)).unwrap(),
            "https://pipkinai.com"
        );
        assert!(target("docs/missing.md", Some(project)).is_err());
        assert!(target("../../etc/passwd", Some(project)).is_err());
        std::os::unix::fs::symlink("/etc/passwd", dir.join("docs/outside.md")).unwrap();
        assert!(target("docs/outside.md", Some(project)).is_err());
        assert!(target("javascript:alert(1)", Some(project)).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
