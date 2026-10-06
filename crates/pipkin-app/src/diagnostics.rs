//! Shareable support metadata: an allowlist, never a scrubbed copy of logs or error text.
//! This is a configured launch-source snapshot, not an attestation of an external server.

use std::path::Path;

use pipkin_core::Mode;

use crate::install;

pub fn prepare(mode: Mode, launch_source: Option<&Path>) -> String {
    let manifest = (mode == Mode::Real)
        .then(|| launch_source.and_then(|path| install::check_engine(path).ok()))
        .flatten();
    render(
        mode,
        manifest.as_ref(),
        install::APP_REVISION,
        install::APP_DIRTY,
    )
}

fn revision(value: &str) -> &str {
    if value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
        value
    } else {
        "unidentified"
    }
}

fn render(
    mode: Mode,
    manifest: Option<&install::Manifest>,
    app_revision: &str,
    dirty: &str,
) -> String {
    let dirty = match dirty {
        "true" => "true",
        "false" => "false",
        _ => "unknown",
    };
    let engine = if mode == Mode::Demo {
        "simulated (no real engine)".to_owned()
    } else {
        manifest.map_or_else(
            || "unidentified (external, missing or rejected launch source)".to_owned(),
            |m| {
                format!(
                    "{} (manifest protocol {})",
                    revision(&m.version),
                    m.protocol
                )
            },
        )
    };
    format!(
        "Pipkin support metadata v1\n\
         app: {}\napp revision: {}\napp dirty: {dirty}\n\
         client protocol: {}\ndatabase schema supported: {}\n\
         build platform: {}/{}\nmode: {}\n\
         configured engine manifest: {engine}\n\
         Scope: launch-source snapshot; not proof of running-server identity, provider authentication or database health.\n\
         Privacy: no logs, error text, paths, session IDs, prompts, attachments, provider configuration or environment values included.\n\
         Review before sharing. Detailed CLI diagnostics are separate and may contain sensitive data.\n",
        install::VERSION,
        revision(app_revision),
        install::PROTOCOL,
        crate::storage::SCHEMA_VERSION,
        std::env::consts::OS,
        std::env::consts::ARCH,
        if mode == Mode::Demo {
            "demo (simulated)"
        } else {
            "real"
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arbitrary_manifest_strings_and_invalid_build_stamp_are_not_copied() {
        let secret = "sk-secret /home/private/project\nAuthorization: Bearer secret";
        let manifest = install::Manifest {
            name: secret.into(),
            version: secret.into(),
            protocol: 8,
            min_client: Some(secret.into()),
            built_at: Some(secret.into()),
        };
        let report = render(Mode::Real, Some(&manifest), secret, secret);
        for fragment in [
            "sk-secret",
            "/home/private",
            "Authorization",
            "Bearer secret",
        ] {
            assert!(!report.contains(fragment), "{report}");
        }
        assert!(report.contains("app revision: unidentified"));
        assert!(report.contains("app dirty: unknown"));
        assert!(report.contains("configured engine manifest: unidentified"));
    }

    #[test]
    fn preserves_only_public_identity_and_marks_unknown_external_identity() {
        let sha = "d2a311097cbcf669e699479587332ae3988a49d0";
        let manifest = install::Manifest {
            name: "private name".into(),
            version: sha.into(),
            protocol: 8,
            min_client: None,
            built_at: None,
        };
        let report = render(Mode::Real, Some(&manifest), sha, "false");
        assert!(report.contains(sha));
        assert!(!report.contains("private name"));
        assert!(render(Mode::Real, None, sha, "true").contains("external, missing or rejected"));
        assert!(render(Mode::Demo, Some(&manifest), sha, "false").contains("no real engine"));
    }

    #[test]
    fn missing_and_hostile_manifest_files_do_not_leak_paths_or_errors() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("secret-project-sk-token");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("engine.json"), "private token invalid JSON").unwrap();
        let report = prepare(Mode::Real, Some(&path));
        assert!(!report.contains("secret-project"));
        assert!(!report.contains("private token"));
        assert!(report.contains("unidentified"));
        assert!(prepare(Mode::Demo, Some(&path)).contains("simulated"));
    }
}
