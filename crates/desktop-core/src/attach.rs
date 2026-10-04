use std::path::Path;

use crate::model::Attachment;

/// Largest file reference accepted for attachment (metadata only; nothing is uploaded).
pub const MAX_ATTACHMENT_BYTES: u64 = 10 * 1024 * 1024;

/// Describe a local path as an attachment, recording a validation error instead of failing.
pub fn describe_attachment(path: &Path) -> Attachment {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let (size, error) = match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            (None, Some("File not found".to_string()))
        }
        Err(e) => (None, Some(format!("Cannot read file: {e}"))),
        Ok(m) if m.is_dir() => (None, Some("Folders cannot be attached".to_string())),
        Ok(m) if m.len() > MAX_ATTACHMENT_BYTES => (
            Some(m.len()),
            Some(format!(
                "Larger than {} MB",
                MAX_ATTACHMENT_BYTES / 1024 / 1024
            )),
        ),
        Ok(m) => (Some(m.len()), None),
    };
    Attachment {
        path: path.display().to_string(),
        name,
        size,
        error,
    }
}
