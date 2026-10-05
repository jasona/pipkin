//! Turn pasted clipboard images into private, durable local attachments.
//! Keep the original encoded bytes: the real adapter already validates image types at send time.

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{Image, ImageFormat};
use pipkin_core::MAX_ATTACHMENT_BYTES;

static NEXT_IMAGE: AtomicU64 = AtomicU64::new(0);

pub fn save(image: &Image, data_dir: &Path) -> Result<PathBuf, String> {
    let ext = match image.format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Gif => "gif",
        ImageFormat::Webp => "webp",
        _ => return Err("Paste a PNG, JPEG, GIF or WebP image.".into()),
    };
    if image.bytes.is_empty() || image.bytes.len() as u64 > MAX_ATTACHMENT_BYTES {
        return Err("The pasted image is empty or exceeds the 10 MB attachment limit.".into());
    }
    let bytes = &image.bytes;
    let valid = match image.format {
        ImageFormat::Png => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        ImageFormat::Jpeg => bytes.starts_with(b"\xff\xd8\xff"),
        ImageFormat::Gif => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        ImageFormat::Webp => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"),
        _ => false,
    };
    if !valid {
        return Err("The clipboard image is not a valid PNG, JPEG, GIF or WebP file.".into());
    }
    let dir = data_dir.join("pasted-images");
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .map_err(|e| format!("Cannot prepare pasted images: {e}"))?;
    let meta =
        fs::symlink_metadata(&dir).map_err(|e| format!("Cannot inspect pasted images: {e}"))?;
    if !meta.is_dir() || meta.permissions().mode() & 0o077 != 0 {
        return Err("Pasted images folder must be private (mode 0700).".into());
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    for _ in 0..8 {
        let seq = NEXT_IMAGE.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("paste-{}-{stamp}-{seq}.{ext}", std::process::id()));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path);
        let mut file = match file {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Cannot save pasted image: {e}")),
        };
        if let Err(e) = file.write_all(&image.bytes).and_then(|_| file.sync_all()) {
            drop(file);
            let _ = fs::remove_file(&path);
            return Err(format!("Cannot save pasted image: {e}"));
        }
        return Ok(path);
    }
    Err("Could not choose a unique name for the pasted image.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasted_image_is_private_and_distinct_and_invalid_formats_are_rejected() {
        let root = std::env::temp_dir().join(format!("pipkin-paste-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let image = Image::from_bytes(
            ImageFormat::Png,
            include_bytes!("../../../../assets/brand/mascot.png").to_vec(),
        );
        let a = save(&image, &root).unwrap();
        let b = save(&image, &root).unwrap();
        assert_ne!(a, b);
        assert_eq!(fs::read(&a).unwrap(), image.bytes);
        assert_eq!(
            fs::metadata(&a).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(a.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert!(save(&Image::from_bytes(ImageFormat::Svg, vec![1]), &root).is_err());
        assert!(save(&Image::from_bytes(ImageFormat::Png, vec![]), &root).is_err());
        assert!(save(&Image::from_bytes(ImageFormat::Png, vec![1, 2]), &root).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
