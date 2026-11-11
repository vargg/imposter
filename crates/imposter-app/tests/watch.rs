//! Watching the collections folder: a burst of writes calls back once, a change under
//! `.git` does not, and dropping the watch stops it. Waits are on a channel with a
//! deadline, never fixed sleeps.
#![cfg(test)]

use std::sync::mpsc;
use std::time::Duration;

use imposter_app::watch::{QUIET, watch};

/// Long enough for any OS to deliver the events, short enough for a failing test.
const DEADLINE: Duration = Duration::from_secs(10);

fn watching(dir: &std::path::Path) -> (imposter_app::watch::Watching, mpsc::Receiver<()>) {
    let (sender, calls) = mpsc::channel();
    let watching = watch(dir, move || {
        let _ = sender.send(());
    })
    .unwrap();
    // Some backends deliver only a moment after the watch starts: write a probe until it
    // is reported, then let its events settle, so each test starts from a quiet watch.
    let probe = dir.join("probe.yaml");
    let deadline = std::time::Instant::now() + DEADLINE;
    let mut n = 0;
    loop {
        n += 1;
        std::fs::write(&probe, format!("n: {n}\n")).unwrap();
        if calls.recv_timeout(QUIET * 2).is_ok() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the watch never reported"
        );
    }
    while calls.recv_timeout(QUIET * 4).is_ok() {}
    (watching, calls)
}

#[test]
fn a_burst_of_writes_calls_back_once() {
    let dir = tempfile::tempdir().unwrap();
    let (_watching, calls) = watching(dir.path());
    std::fs::create_dir_all(dir.path().join("api")).unwrap();
    for n in 0..5 {
        std::fs::write(dir.path().join(format!("api/r{n}.yaml")), "id: x\n").unwrap();
    }
    calls.recv_timeout(DEADLINE).expect("a change is reported");
    assert!(
        calls.recv_timeout(QUIET * 4).is_err(),
        "the burst was reported once"
    );
}

#[test]
fn hidden_directories_are_not_a_change_and_dropping_stops_it() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".git/objects")).unwrap();
    let (watching, calls) = watching(dir.path());
    std::fs::write(dir.path().join(".git/objects/abc"), "blob").unwrap();
    std::fs::write(dir.path().join(".git/index"), "index").unwrap();
    assert!(
        calls.recv_timeout(QUIET * 4).is_err(),
        "a commit's bookkeeping is not a change"
    );

    drop(watching);
    std::fs::write(dir.path().join("after.yaml"), "id: y\n").unwrap();
    assert!(
        calls.recv_timeout(QUIET * 4).is_err(),
        "nothing is watched after the drop"
    );
}

#[test]
fn a_missing_folder_is_created_and_watched() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("collections");
    // The probe in `watching` is reported, so the new folder is watched.
    let (_watching, _calls) = watching(&folder);
    assert!(folder.is_dir());
}

/// The default folder is `~/.imposter/collections`: a dotted directory above the folder
/// does not make what changes inside it hidden (on Windows it did, and every change was
/// ignored).
#[test]
fn a_folder_under_a_dotted_directory_is_watched() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join(".imposter").join("collections");
    let (_watching, calls) = watching(&folder);
    std::fs::write(folder.join("login.yaml"), "id: z\n").unwrap();
    calls.recv_timeout(DEADLINE).expect("a change is reported");
}
