use std::path::Path;

pub(crate) fn hive_on_path(env: &[String]) -> String {
    let mut path_env = "";
    for kv in env {
        if let Some(rest) = kv.strip_prefix("PATH=") {
            path_env = rest;
        }
    }
    for dir in std::env::split_paths(path_env) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let p = dir.join("hive");
        if is_executable_file(&p) {
            return p.to_string_lossy().into_owned();
        }
    }
    String::new()
}

pub(crate) fn preflight_line(found: String, cli: String) -> String {
    if !found.is_empty() {
        format!("preflight: hive on worker PATH: {found}")
    } else if !cli.is_empty() {
        format!("preflight: hive missing from worker PATH; worker must use $HIVE_CLI={cli}")
    } else {
        "preflight: hive missing from worker PATH and no HIVE_CLI set".into()
    }
}

pub(crate) fn preflight_hive_cli(found: String, cli: String) -> anyhow::Result<()> {
    if !found.is_empty() {
        return Ok(());
    }
    if !cli.is_empty() {
        if is_executable_file(Path::new(&cli)) {
            return Ok(());
        }
        return Err(crate::util::fail(
            1,
            format!(
                "setup-error: hive is missing from the worker PATH and HIVE_CLI={cli} is not an executable file; the worker was not started so no work is lost"
            ),
        )
        .into());
    }
    Err(crate::util::fail(
        1,
        "setup-error: hive is missing from the worker PATH and no HIVE_CLI is set; the worker was not started so no work is lost",
    )
    .into())
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn zero_space(_: &str) -> anyhow::Result<u64> {
        Ok(0)
    }

    #[cfg(unix)]
    fn write_exe(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        fs::write(path, "#!/bin/sh\n").unwrap();
        let mut perm = fs::metadata(path).unwrap().permissions();
        perm.set_mode(0o755);
        fs::set_permissions(path, perm).unwrap();
    }

    #[test]
    fn hive_on_path_finds_executable_on_worker_path() {
        let _serial = crate::test_helpers::serial();
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("hive");
        write_exe(&bin);
        let empty = tempfile::tempdir().unwrap();
        let path = std::env::join_paths([empty.path(), dir.path()]).unwrap();
        let env = vec![format!("PATH={}", path.to_string_lossy()), "HOME=/x".into()];
        assert_eq!(hive_on_path(&env), bin.to_string_lossy());
        assert_eq!(
            hive_on_path(&[format!("PATH={}", empty.path().display())]),
            ""
        );
        assert_eq!(
            preflight_line("".into(), "/opt/hive".into()),
            "preflight: hive missing from worker PATH; worker must use $HIVE_CLI=/opt/hive"
        );
        assert_eq!(
            preflight_line(bin.to_string_lossy().into_owned(), "".into()),
            format!("preflight: hive on worker PATH: {}", bin.display())
        );
    }

    #[test]
    fn hive_on_path_missing_is_setup_error() {
        let _serial = crate::test_helpers::serial();
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("hive");
        write_exe(&exe);
        assert!(preflight_hive_cli(exe.to_string_lossy().into_owned(), "".into()).is_ok());
        assert!(preflight_hive_cli("".into(), exe.to_string_lossy().into_owned()).is_ok());
        let err = preflight_hive_cli("".into(), "".into())
            .unwrap_err()
            .to_string();
        assert!(err.contains("setup-error"));
        assert!(err.contains("no HIVE_CLI"));
        let err = preflight_hive_cli(
            "".into(),
            dir.path().join("nope").to_string_lossy().into_owned(),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("setup-error"));
        assert!(err.contains("not an executable"));
        assert!(preflight_hive_cli("".into(), dir.path().to_string_lossy().into_owned()).is_err());
    }

    #[test]
    fn setup_errors_share_prefix() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        let missing = tempfile::tempdir().unwrap().path().join("absent");
        let empty_dir = tempfile::tempdir().unwrap();
        let empty = empty_dir.path().join("empty");
        fs::write(&empty, []).unwrap();
        let prev = *crate::tmpspace::FREE_SPACE.read().unwrap();
        *crate::tmpspace::FREE_SPACE.write().unwrap() = zero_space;
        let cases = vec![
            (
                "preflightHiveCLI no cli",
                preflight_hive_cli("".into(), "".into()),
            ),
            (
                "preflightHiveCLI bad cli",
                preflight_hive_cli("".into(), missing.to_string_lossy().into_owned()),
            ),
            (
                "preflightRunKey missing",
                crate::exec::preflight_run_key(&missing.to_string_lossy()),
            ),
            (
                "preflightRunKey empty",
                crate::exec::preflight_run_key(&empty.to_string_lossy()),
            ),
            (
                "preflightTmpSpace unusable",
                crate::tmpspace::preflight_tmp_space(
                    &format!("{}/x", missing.to_string_lossy()),
                    1,
                ),
            ),
            (
                "preflightTmpSpace low",
                crate::tmpspace::preflight_tmp_space(
                    &tempfile::tempdir().unwrap().path().display().to_string(),
                    1,
                ),
            ),
            (
                "remoteKeyCheckResult",
                crate::exec::remote_key_check_result(
                    "/j/run.key",
                    "host",
                    "after upload",
                    1,
                    "",
                    "no such file",
                ),
            ),
            (
                "remoteCLICheckResult",
                crate::exec::remote_cli_check_result("/opt/hive", "host", 1, "", ""),
            ),
        ];
        *crate::tmpspace::FREE_SPACE.write().unwrap() = prev;
        for (name, err) in cases {
            let err = err.unwrap_err().to_string();
            assert!(
                err.starts_with("setup-error:"),
                "{name}: {err:?} does not start with \"setup-error:\""
            );
        }
    }
}
