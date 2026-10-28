//! Background work for a UI thread that never blocks.
//!
//! The UI starts work with [`Bridge::spawn`]; each task's result arrives on a channel that the UI
//! drains once per frame, and the `wake` callback asks the UI for a repaint. Every task gets a
//! [`CancellationToken`]; shutdown cancels all of them and waits for the work to finish.

use std::collections::HashMap;
use std::future::Future;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tokio::runtime::{Handle, Runtime};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TaskId(u64);

type Wake = Arc<dyn Fn() + Send + Sync>;

struct Entry {
    token: CancellationToken,
    join: Option<JoinHandle<()>>,
}

/// Owns every running task. Tasks deregister themselves when they finish, panic or are aborted.
#[derive(Default)]
struct Registry {
    state: Mutex<RegistryState>,
    idle: Condvar,
}

#[derive(Default)]
struct RegistryState {
    next_id: u64,
    tasks: HashMap<TaskId, Entry>,
}

impl Registry {
    fn lock(&self) -> std::sync::MutexGuard<'_, RegistryState> {
        // A panic inside a task never holds this lock, so poisoning carries no broken invariant.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn remove(&self, id: TaskId) {
        let mut state = self.lock();
        state.tasks.remove(&id);
        if state.tasks.is_empty() {
            self.idle.notify_all();
        }
    }
}

/// Removes the task from the registry however its future ends (completion, panic, abort).
struct Deregister {
    registry: Arc<Registry>,
    id: TaskId,
}

impl Drop for Deregister {
    fn drop(&mut self) {
        self.registry.remove(self.id);
    }
}

pub struct Bridge<M> {
    runtime: Option<Runtime>,
    registry: Arc<Registry>,
    sender: Sender<M>,
    receiver: Receiver<M>,
    wake: Wake,
}

impl<M: Send + 'static> Bridge<M> {
    /// Start the background runtime. `wake` is called after each result is queued.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> std::io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .thread_name("imposter-worker")
            .enable_all()
            .build()?;
        let (sender, receiver) = mpsc::channel();
        Ok(Self {
            runtime: Some(runtime),
            registry: Arc::default(),
            sender,
            receiver,
            wake: Arc::new(wake),
        })
    }

    /// Replace the wake callback, for example once the UI context exists.
    pub fn set_wake(&mut self, wake: impl Fn() + Send + Sync + 'static) {
        self.wake = Arc::new(wake);
    }

    fn handle(&self) -> &Handle {
        // `runtime` is only taken in `Drop`, after which no method can run.
        self.runtime
            .as_ref()
            .map(Runtime::handle)
            .expect("runtime lives until drop")
    }

    /// Run `op` in the background. Its output is delivered by [`Self::drain`].
    pub fn spawn<F, Fut>(&self, op: F) -> TaskId
    where
        F: FnOnce(CancellationToken) -> Fut,
        Fut: Future<Output = M> + Send + 'static,
    {
        let token = CancellationToken::new();
        let future = op(token.clone());
        let sender = self.sender.clone();
        let wake = Arc::clone(&self.wake);
        // Hold the lock across spawn so a task that finishes at once cannot deregister before
        // it was registered.
        let mut state = self.registry.lock();
        state.next_id += 1;
        let id = TaskId(state.next_id);
        let guard = Deregister {
            registry: Arc::clone(&self.registry),
            id,
        };
        let join = self.handle().spawn(async move {
            let _guard = guard;
            let output = future.await;
            // The receiver only disappears during shutdown, when results are no longer wanted.
            if sender.send(output).is_ok() {
                wake();
            }
        });
        state.tasks.insert(
            id,
            Entry {
                token,
                join: Some(join),
            },
        );
        id
    }

    /// Deliver intermediate events from a long-running service through the frame channel.
    pub fn notifier(&self) -> impl Fn(M) + Send + 'static {
        let sender = self.sender.clone();
        let wake = Arc::clone(&self.wake);
        move |message| {
            if sender.send(message).is_ok() {
                wake();
            }
        }
    }

    /// Ask one task to stop. The task decides how (see its use of the token).
    pub fn cancel(&self, id: TaskId) {
        if let Some(entry) = self.registry.lock().tasks.get(&id) {
            entry.token.cancel();
        }
    }

    pub fn is_running(&self, id: TaskId) -> bool {
        self.registry.lock().tasks.contains_key(&id)
    }

    pub fn running(&self) -> usize {
        self.registry.lock().tasks.len()
    }

    /// Results that arrived since the last call, in completion order.
    pub fn drain(&self) -> Vec<M> {
        self.receiver.try_iter().collect()
    }

    /// Block until no task is running or `timeout` passes. Returns whether all tasks finished.
    /// For shutdown and tests; never call it from a frame.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = self.registry.lock();
        while !state.tasks.is_empty() {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            state = self
                .registry
                .idle
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }

    /// Cancel every task and wait up to `grace` for all of them to finish; abort the rest.
    /// Returns whether everything finished within `grace`.
    pub fn shutdown(&self, grace: Duration) -> bool {
        for entry in self.registry.lock().tasks.values() {
            entry.token.cancel();
        }
        let finished = self.wait_idle(grace);
        if !finished {
            let joins: Vec<_> = self
                .registry
                .lock()
                .tasks
                .values_mut()
                .filter_map(|entry| entry.join.take())
                .collect();
            tracing::warn!(count = joins.len(), "aborting tasks that ignored shutdown");
            for join in joins {
                join.abort();
            }
            self.wait_idle(grace);
        }
        finished
    }

    /// Run a future to completion on the background runtime, blocking the caller.
    /// For startup and shutdown only, before or after the UI runs frames.
    pub fn block_on<T>(&self, future: impl Future<Output = T>) -> T {
        self.handle().block_on(future)
    }

    /// A handle for code that must spawn its own background work (for example the database).
    pub fn runtime_handle(&self) -> Handle {
        self.handle().clone()
    }
}

impl<M> Drop for Bridge<M> {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::sync::oneshot;

    use super::*;

    const WAIT: Duration = Duration::from_secs(5);

    fn bridge() -> (Bridge<&'static str>, Arc<AtomicUsize>) {
        let wakes = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&wakes);
        let bridge = Bridge::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        (bridge, wakes)
    }

    #[test]
    fn results_arrive_on_drain_and_wake_the_ui() {
        let (bridge, wakes) = bridge();

        let id = bridge.spawn(|_| async { "done" });

        assert!(bridge.wait_idle(WAIT));
        assert_eq!(bridge.drain(), ["done"]);
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        assert!(!bridge.is_running(id));
        assert_eq!(bridge.drain(), Vec::<&str>::new());
    }

    #[test]
    fn cancel_reaches_the_task_token() {
        let (bridge, _) = bridge();
        let (started_tx, started_rx) = oneshot::channel();

        let id = bridge.spawn(|token| async move {
            started_tx.send(()).unwrap();
            token.cancelled().await;
            "cancelled"
        });
        bridge.block_on(started_rx).unwrap();
        assert!(bridge.is_running(id));
        bridge.cancel(id);

        assert!(bridge.wait_idle(WAIT));
        assert_eq!(bridge.drain(), ["cancelled"]);
    }

    #[test]
    fn a_panicking_task_still_deregisters() {
        let (bridge, wakes) = bridge();

        bridge.spawn(|_| async { panic!("boom") });

        assert!(bridge.wait_idle(WAIT));
        assert_eq!(bridge.running(), 0);
        assert_eq!(bridge.drain(), Vec::<&str>::new());
        assert_eq!(wakes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn shutdown_cancels_and_waits_for_cooperative_tasks() {
        let (bridge, _) = bridge();
        let (started_tx, started_rx) = oneshot::channel();
        bridge.spawn(|token| async move {
            started_tx.send(()).unwrap();
            token.cancelled().await;
            "stopped"
        });
        bridge.block_on(started_rx).unwrap();

        assert!(bridge.shutdown(WAIT));
        assert_eq!(bridge.drain(), ["stopped"]);
    }

    #[test]
    fn shutdown_lets_work_that_ignores_the_token_finish() {
        let (bridge, _) = bridge();
        let (release_tx, release_rx) = oneshot::channel::<()>();
        bridge.spawn(|_| async move {
            release_rx.await.unwrap();
            "saved"
        });
        release_tx.send(()).unwrap();

        assert!(bridge.shutdown(WAIT));
        assert_eq!(bridge.drain(), ["saved"]);
    }

    #[test]
    fn shutdown_aborts_tasks_that_outlive_the_grace_period() {
        let (bridge, _) = bridge();
        let (_never_tx, never_rx) = oneshot::channel::<()>();
        bridge.spawn(|_| async move {
            never_rx.await.ok();
            "unreachable"
        });

        assert!(!bridge.shutdown(Duration::from_millis(50)));
        assert_eq!(bridge.running(), 0);
        assert_eq!(bridge.drain(), Vec::<&str>::new());
    }

    #[test]
    fn wait_idle_times_out_while_work_runs() {
        let (bridge, _) = bridge();
        let (release_tx, release_rx) = oneshot::channel::<()>();
        bridge.spawn(|_| async move {
            release_rx.await.ok();
            "late"
        });

        assert!(!bridge.wait_idle(Duration::from_millis(20)));
        release_tx.send(()).unwrap();
        assert!(bridge.wait_idle(WAIT));
    }
}
