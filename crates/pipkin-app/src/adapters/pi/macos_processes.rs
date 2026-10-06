//! macOS process ownership: kernel uid + exact NUL-delimited environment identity.
//! Never parse `ps e` text (spaces/argv can impersonate an environment variable), and never log
//! the process argument/environment buffer. Uninspectable processes are not signal targets.

fn owns_profile(bytes: &[u8], directory: &[u8], identity: &[u8]) -> bool {
    let Some(count) = bytes
        .get(..4)
        .and_then(|b| <[u8; 4]>::try_from(b).ok())
        .map(i32::from_ne_bytes)
    else {
        return false;
    };
    if !(1..=8192).contains(&count) {
        return false;
    }
    let mut rest = &bytes[4..];
    // KERN_PROCARGS2: argc, executable path, NUL padding, argc argv strings, environment.
    let Some(end) = rest.iter().position(|b| *b == 0) else {
        return false;
    };
    rest = &rest[end + 1..];
    while rest.first() == Some(&0) {
        rest = &rest[1..];
    }
    for _ in 0..count {
        let Some(end) = rest.iter().position(|b| *b == 0) else {
            return false;
        };
        rest = &rest[end + 1..];
    }
    let (mut dir, mut id) = (false, false);
    while let Some(end) = rest.iter().position(|b| *b == 0) {
        let value = &rest[..end];
        if value.is_empty() {
            break;
        }
        dir |= value == directory;
        id |= value == identity;
        rest = &rest[end + 1..];
    }
    dir && id
}

#[cfg(target_os = "macos")]
pub(super) fn profile_pids(server_dir: &std::path::Path, server_id: &str) -> Vec<u32> {
    use std::mem::{MaybeUninit, size_of};
    use std::os::unix::ffi::OsStrExt;
    // SAFETY: read-only libproc enumeration; count query has no output buffer.
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return Vec::new();
    }
    let mut pids = vec![0_i32; count as usize + 256];
    // SAFETY: buffer holds the advertised byte length, with room for process churn.
    let count = unsafe {
        libc::proc_listallpids(
            pids.as_mut_ptr().cast(),
            (pids.len() * size_of::<i32>()) as i32,
        )
    };
    if count <= 0 {
        return Vec::new();
    }
    pids.truncate((count as usize).min(pids.len()));
    let mut directory = b"PI_SERVER_DIR=".to_vec();
    directory.extend_from_slice(server_dir.as_os_str().as_bytes());
    let identity = format!("PI_SERVER_ID={server_id}");
    // SAFETY: geteuid has no preconditions.
    let uid = unsafe { libc::geteuid() };
    let me = std::process::id();
    let mut found = Vec::new();
    for pid in pids.into_iter().filter(|p| *p > 0 && *p as u32 != me) {
        let mut info = MaybeUninit::<libc::proc_bsdinfo>::uninit();
        // SAFETY: output points to a correctly sized proc_bsdinfo; read only after full success.
        let read = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast(),
                size_of::<libc::proc_bsdinfo>() as i32,
            )
        };
        if read != size_of::<libc::proc_bsdinfo>() as i32 {
            continue;
        }
        // SAFETY: proc_pidinfo wrote the complete structure above.
        if unsafe { info.assume_init() }.pbi_uid != uid {
            continue;
        }
        let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
        let mut length = 0_usize;
        // SAFETY: three-element MIB and valid output length; no data buffer for size query.
        if unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                3,
                std::ptr::null_mut(),
                &mut length,
                std::ptr::null_mut(),
                0,
            )
        } != 0
            || !(4..=4 * 1024 * 1024).contains(&length)
        {
            continue;
        }
        let mut buffer = vec![0_u8; length];
        // SAFETY: buffer holds length bytes, MIB and in/out length remain valid.
        if unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                3,
                buffer.as_mut_ptr().cast(),
                &mut length,
                std::ptr::null_mut(),
                0,
            )
        } != 0
            || length > buffer.len()
        {
            continue;
        }
        buffer.truncate(length);
        if owns_profile(&buffer, &directory, identity.as_bytes()) {
            found.push(pid as u32);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::owns_profile;

    fn buffer(args: &[&[u8]], env: &[&[u8]]) -> Vec<u8> {
        let mut out = (args.len() as i32).to_ne_bytes().to_vec();
        out.extend_from_slice(b"/usr/local/bin/node\0\0\0");
        for value in args.iter().chain(env.iter()) {
            out.extend_from_slice(value);
            out.push(0);
        }
        out.push(0);
        out
    }

    #[test]
    fn exact_environment_only_not_argv_prefixes_or_spaces() {
        let dir = "PI_SERVER_DIR=/a directory/Unicode-λ".as_bytes();
        let id = b"PI_SERVER_ID=ours";
        assert!(owns_profile(
            &buffer(&[b"node", b"server"], &[dir, id]),
            dir,
            id
        ));
        assert!(!owns_profile(&buffer(&[b"node", dir, id], &[]), dir, id));
        assert!(!owns_profile(
            &buffer(&[b"node"], &[dir, b"PI_SERVER_ID=ours-other"]),
            dir,
            id
        ));
        assert!(!owns_profile(
            &buffer(&[b"node"], &[b"COMMENT=PI_SERVER_ID=ours", dir]),
            dir,
            id
        ));
        assert!(!owns_profile(&buffer(&[b"node"], &[id]), dir, id));
    }

    #[test]
    fn malformed_and_truncated_buffers_fail_closed() {
        for bytes in [
            &[][..],
            &[0, 0, 0][..],
            &0_i32.to_ne_bytes()[..],
            &(-1_i32).to_ne_bytes()[..],
            b"\x01\0\0\0no terminator",
        ] {
            assert!(!owns_profile(bytes, b"DIR=x", b"ID=y"));
        }
        let valid = buffer(&[b"node"], &[b"DIR=x", b"ID=y"]);
        assert!(!owns_profile(&valid[..valid.len() - 2], b"DIR=x", b"ID=y"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn kernel_discovery_selects_only_the_disposable_owned_child() {
        use std::process::Command;
        let dir = tempfile::tempdir().unwrap();
        let id = "pipkin-macos-discovery-test";
        let mut owned = Command::new("/bin/sleep")
            .arg("30")
            .env("PI_SERVER_DIR", dir.path())
            .env("PI_SERVER_ID", id)
            .spawn()
            .unwrap();
        let mut unrelated = Command::new("/bin/sleep")
            .arg("30")
            .env("PI_SERVER_DIR", dir.path())
            .env("PI_SERVER_ID", "different")
            .spawn()
            .unwrap();
        let mut found = Vec::new();
        for _ in 0..20 {
            found = super::profile_pids(dir.path(), id);
            if found.contains(&owned.id()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        // Clean up children by their direct handles before asserting; never signal discovered pids.
        let _ = owned.kill();
        let _ = owned.wait();
        let _ = unrelated.kill();
        let _ = unrelated.wait();
        assert!(
            found.contains(&owned.id()),
            "owned child was not discoverable"
        );
        assert!(!found.contains(&unrelated.id()));
        assert!(!found.contains(&std::process::id()));
    }
}
