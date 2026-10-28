//! Environment behavior is tested in child processes; the test runner's environment stays intact.
#![cfg(test)]

use imposter_app::paths;

#[test]
fn child_profile() {
    let Some(root) = std::env::var_os("IMPOSTER_PROFILE_TEST") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let explicit = std::env::var_os("IMPOSTER_DATA_DIR").is_some();
    let store = paths::open_profile().unwrap();
    assert_eq!(
        store.path(),
        root.join(if explicit {
            "explicit/imposter.db"
        } else {
            ".imposter/imposter.db"
        })
    );
    assert_eq!(paths::log_filter(), if explicit { "warn" } else { "info" });
    store.close().unwrap();
    assert!(!root.join("old").exists());
}

#[test]
fn only_new_environment_variables_are_recognized() {
    for explicit in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args(["--exact", "child_profile", "--nocapture"])
            .env("IMPOSTER_PROFILE_TEST", dir.path())
            .env("HOME", dir.path())
            .env("USERPROFILE", dir.path())
            .env("POSTER_DATA_DIR", dir.path().join("old"))
            .env("POSTER_LOG", "trace")
            .env_remove("IMPOSTER_DATA_DIR")
            .env_remove("IMPOSTER_LOG");
        if explicit {
            child
                .env("IMPOSTER_DATA_DIR", dir.path().join("explicit"))
                .env("IMPOSTER_LOG", "warn");
        }
        let result = child.output().unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
