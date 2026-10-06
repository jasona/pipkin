use std::path::{Path, PathBuf};
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let revision = git(&root, &["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let dirty = git(
        &root,
        &["status", "--porcelain", "--untracked-files=normal"],
    )
    .map_or(
        "unknown",
        |status| if status.is_empty() { "false" } else { "true" },
    );
    println!("cargo:rustc-env=PIPKIN_APP_REVISION={revision}");
    println!("cargo:rustc-env=PIPKIN_APP_DIRTY={dirty}");
    // Follow both ordinary repositories and worktrees. Source changes and new commits
    // must invalidate the stamp even when Cargo would otherwise reuse the binary.
    for name in ["HEAD", "index"] {
        if let Some(path) = git(&root, &["rev-parse", "--git-path", name]) {
            println!("cargo:rerun-if-changed={}", root.join(path).display());
        }
    }
    if let Some(reference) = git(&root, &["symbolic-ref", "-q", "HEAD"])
        && let Some(path) = git(&root, &["rev-parse", "--git-path", &reference])
    {
        println!("cargo:rerun-if-changed={}", root.join(path).display());
    }
    if let Some(files) = git(&root, &["ls-files"]) {
        for file in files.lines() {
            println!("cargo:rerun-if-changed={}", root.join(file).display());
        }
    }
    println!("cargo:rerun-if-changed={}", root.join("crates").display());
}
