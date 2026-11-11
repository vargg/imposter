//! Watching the collections folder for changes made outside the app (a `git pull`, another
//! editor, an agent). Events come in bursts (a checkout touches many files, an editor
//! writes through a temporary file), so they are coalesced: the callback runs once after
//! the folder has been quiet for [`QUIET`]. What changed is for the store to find out
//! (`files::reload`); the watcher only says that something may have.

use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// How long the folder must be quiet before the callback runs.
pub const QUIET: Duration = Duration::from_millis(250);

#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    #[error("Cannot create the collections folder {0}: {1}")]
    Create(String, #[source] std::io::Error),
    #[error("Cannot watch the collections folder: {0}")]
    Watch(#[from] notify::Error),
}

/// A folder being watched; dropping it stops the watching.
pub struct Watching {
    watcher: Option<RecommendedWatcher>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Watching {
    fn drop(&mut self) {
        // The watcher holds the only sender: dropping it ends the thread's loop.
        self.watcher.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Whether an event can change what the folder holds: not a read, and not only under a
/// hidden directory (`.git`), which a commit or fetch changes on every run. Only the part
/// of a path inside the folder is judged, found under any of `roots`: the folder as given
/// and as canonical, since Windows reports the first (`C:\Users\RUNNER~1\…`) and
/// canonicalizing gives another (`\\?\C:\Users\runneradmin\…`). A path under none of
/// them is a change: judged whole, every event under `~/.imposter/collections` was hidden.
fn matters(event: &Event, roots: &[PathBuf]) -> bool {
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    event.paths.is_empty()
        || event.paths.iter().any(|path| {
            roots
                .iter()
                .find_map(|root| path.strip_prefix(root).ok())
                .is_none_or(|inside| {
                    !inside.components().any(|c| {
                        matches!(c, Component::Normal(name) if name.to_string_lossy().starts_with('.'))
                    })
                })
        })
}

/// Watch `dir` and everything under it, creating it when missing; `on_change` runs on the
/// watcher's own thread, once per burst of changes.
pub fn watch(dir: &Path, on_change: impl Fn() + Send + 'static) -> Result<Watching, WatchError> {
    std::fs::create_dir_all(dir).map_err(|e| WatchError::Create(dir.display().to_string(), e))?;
    // Events name canonical paths on some systems (/private/var on macOS), and the path as
    // given on others (Windows).
    let mut roots = vec![dir.to_owned()];
    roots.extend(dir.canonicalize().ok().filter(|canonical| canonical != dir));
    let (sender, events) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher = notify::recommended_watcher(sender)?;
    watcher.watch(dir, RecursiveMode::Recursive)?;
    let thread = std::thread::Builder::new()
        .name("imposter-watch".to_owned())
        .spawn(move || {
            while let Ok(event) = events.recv() {
                // An error from the watcher may mean events were lost: check the folder.
                if event.as_ref().is_ok_and(|e| !matters(e, &roots)) {
                    continue;
                }
                // Coalesce the burst: wait until nothing has come for QUIET.
                loop {
                    match events.recv_timeout(QUIET) {
                        Ok(_) => {}
                        Err(mpsc::RecvTimeoutError::Timeout) => break,
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
                on_change();
            }
        })
        .map_err(|e| WatchError::Create(dir.display().to_string(), e))?;
    Ok(Watching {
        watcher: Some(watcher),
        thread: Some(thread),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::CreateKind;

    fn created(path: &str) -> Event {
        Event::new(EventKind::Create(CreateKind::File)).add_path(PathBuf::from(path))
    }

    /// Windows names events by the folder as given, which differs from its canonical
    /// form; the folder's own dotted ancestors must not make every event hidden.
    #[test]
    fn events_are_judged_inside_the_folder_as_given_or_canonical() {
        let roots = [
            PathBuf::from("/home/me/.imposter/collections"),
            PathBuf::from("/real/me/.imposter/collections"),
        ];
        assert!(matters(
            &created("/home/me/.imposter/collections/api/login.yaml"),
            &roots
        ));
        assert!(matters(
            &created("/real/me/.imposter/collections/api/login.yaml"),
            &roots
        ));
        assert!(!matters(
            &created("/home/me/.imposter/collections/.git/index"),
            &roots
        ));
        assert!(
            matters(&created("/elsewhere/.hidden/x.yaml"), &roots),
            "a path under no root is a change, not judged by its ancestors"
        );
    }
}
