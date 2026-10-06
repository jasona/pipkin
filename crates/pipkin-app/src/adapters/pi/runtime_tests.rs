use super::{bundled_runtime_bin, runtime_path};
use std::path::Path;

#[test]
fn runtime_is_explicit_and_missing_declared_payload_never_falls_back() {
    let root = tempfile::tempdir().unwrap();
    let engine = root.path().join("engine");
    std::fs::create_dir(&engine).unwrap();
    assert!(bundled_runtime_bin(&engine).unwrap().is_none());
    std::fs::write(engine.join("engine.json"), r#"{"version":"fixture"}"#).unwrap();
    assert!(bundled_runtime_bin(&engine).unwrap().is_none());
    std::fs::write(
        engine.join("engine.json"),
        r#"{"requiresBundledNode":true}"#,
    )
    .unwrap();
    assert!(bundled_runtime_bin(&engine).is_err());
    let runtime = root.path().join("runtime");
    std::fs::create_dir_all(runtime.join("bin")).unwrap();
    std::fs::write(runtime.join("runtime.json"), "{}").unwrap();
    std::fs::write(runtime.join("bin/node"), "fixture").unwrap();
    assert_eq!(
        bundled_runtime_bin(&engine).unwrap(),
        Some(runtime.join("bin"))
    );
    std::fs::remove_file(runtime.join("bin/node")).unwrap();
    assert!(bundled_runtime_bin(&engine).is_err());
}

#[test]
fn diagnostics_use_declared_runtime_without_a_global_node_dependency() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let engine = root.path().join("engine");
    let runtime = root.path().join("runtime");
    std::fs::create_dir(&engine).unwrap();
    std::fs::create_dir_all(runtime.join("bin")).unwrap();
    std::fs::write(
        engine.join("engine.json"),
        r#"{"requiresBundledNode":true}"#,
    )
    .unwrap();
    std::fs::write(runtime.join("runtime.json"), "{}").unwrap();
    std::fs::write(
        runtime.join("bin/node"),
        "#!/bin/sh\nprintf 'v22.23.3\\n'\n",
    )
    .unwrap();
    std::fs::set_permissions(
        runtime.join("bin/node"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert_eq!(
        crate::install::node_version(Some(&engine)).unwrap(),
        (22, 23, 3)
    );
    std::fs::remove_file(runtime.join("bin/node")).unwrap();
    assert!(crate::install::node_version(Some(&engine)).is_err());
}

#[test]
fn bundled_path_precedes_bare_or_custom_path_without_mutating_process_environment() {
    let bin = Path::new("/Applications/Pipkin with spaces.app/Contents/lib/pipkin/runtime/bin");
    assert_eq!(
        std::env::split_paths(
            &runtime_path(bin, Some(std::ffi::OsStr::new("/usr/bin:/bin"))).unwrap()
        )
        .collect::<Vec<_>>(),
        vec![bin.to_path_buf(), "/usr/bin".into(), "/bin".into()]
    );
    assert_eq!(runtime_path(bin, None).unwrap(), bin.as_os_str());
}
