//! Per-request dirty tracking and serialized saving.
//!
//! The saver is plain state owned by the UI. The UI queues edits with [`Saver::update`], takes
//! one write at a time with [`Saver::next_write`], performs it in the background and reports
//! the result with [`Saver::finish`]. Failed writes keep their snapshot for retry or explicit
//! discard; reverting an edit to the saved state clears it.

use std::collections::{BTreeMap, HashMap};

use imposter_core::ids::RequestId;
use imposter_core::model::Request;

#[derive(Debug, Default)]
pub struct Saver {
    /// What the database holds, as far as this session knows.
    saved: HashMap<RequestId, Request>,
    /// The newest unsaved snapshot of each edited request. Ordered for deterministic writes.
    pending: BTreeMap<RequestId, Request>,
    /// The snapshot being written now, if any.
    in_flight: Option<(RequestId, Request)>,
    /// The last failure per request, until it is saved or discarded.
    failures: HashMap<RequestId, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Saved,
    Dirty,
    Saving,
    Failed,
}

impl Saver {
    /// Register the persisted state of a request the editor just opened.
    pub fn remember(&mut self, request: &Request) {
        if let Some(id) = request.id
            && !self.pending.contains_key(&id)
        {
            self.saved.insert(id, request.clone());
        }
    }

    pub fn baseline(&self, id: RequestId) -> Option<&Request> {
        self.saved.get(&id)
    }

    /// Keep tree operations (rename, move) in the saved and pending snapshots, so a later save
    /// of editor changes does not undo them.
    pub fn adopt_metadata(&mut self, id: RequestId, apply: impl Fn(&mut Request)) {
        for snapshot in self
            .saved
            .get_mut(&id)
            .into_iter()
            .chain(self.pending.get_mut(&id))
        {
            apply(snapshot);
        }
    }

    /// Record the editor's current state. Drafts without an id are not tracked here.
    pub fn update(&mut self, request: &Request) {
        let Some(id) = request.id else { return };
        let writing = self
            .in_flight
            .as_ref()
            .is_some_and(|(flight, _)| *flight == id);
        if !writing && self.saved.get(&id) == Some(request) {
            self.pending.remove(&id);
            self.failures.remove(&id);
        } else {
            self.pending.insert(id, request.clone());
        }
    }

    pub fn pending(&self, id: RequestId) -> Option<&Request> {
        self.pending.get(&id)
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty() || self.in_flight.is_some()
    }

    pub fn state(&self, id: RequestId) -> State {
        if self
            .in_flight
            .as_ref()
            .is_some_and(|(flight, _)| *flight == id)
        {
            State::Saving
        } else if self.failures.contains_key(&id) {
            State::Failed
        } else if self.pending.contains_key(&id) {
            State::Dirty
        } else {
            State::Saved
        }
    }

    pub fn failure(&self, id: RequestId) -> Option<&str> {
        self.failures.get(&id).map(String::as_str)
    }

    pub fn failures(&self) -> impl Iterator<Item = (RequestId, &str)> {
        self.failures
            .iter()
            .map(|(id, message)| (*id, message.as_str()))
    }

    /// Drop unsaved edits of one request (`Some`) or all (`None`) after the user confirmed.
    pub fn discard(&mut self, id: Option<RequestId>) {
        if let Some(id) = id {
            self.pending.remove(&id);
            self.failures.remove(&id);
        } else {
            self.pending.clear();
            self.failures.clear();
        }
    }

    /// The next write to perform: the given request only, or any. `None` while a write is in
    /// flight, so writes never overlap.
    pub fn next_write(&mut self, only: Option<RequestId>) -> Option<(RequestId, Request)> {
        if self.in_flight.is_some() {
            return None;
        }
        let id = match only {
            Some(id) => self.pending.contains_key(&id).then_some(id)?,
            None => *self.pending.keys().next()?,
        };
        let snapshot = self.pending.get(&id)?.clone();
        self.in_flight = Some((id, snapshot.clone()));
        Some((id, snapshot))
    }

    /// Report the result of the write handed out by [`Self::next_write`].
    pub fn finish(&mut self, result: Result<(), String>) {
        let Some((id, snapshot)) = self.in_flight.take() else {
            return;
        };
        match result {
            Ok(()) => {
                self.failures.remove(&id);
                if self.pending.get(&id) == Some(&snapshot) {
                    self.pending.remove(&id);
                }
                self.saved.insert(id, snapshot);
            }
            Err(message) => {
                self.failures.insert(id, message);
            }
        }
    }

    /// Forget a request that was deleted.
    pub fn forget(&mut self, id: RequestId) {
        self.saved.remove(&id);
        self.pending.remove(&id);
        self.failures.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(id: i64, url: &str) -> Request {
        let mut request = Request {
            id: Some(RequestId(id)),
            ..Request::default()
        };
        request.template.url = url.to_owned();
        request
    }

    #[test]
    fn edits_are_dirty_and_reverting_clears_them() {
        let mut saver = Saver::default();
        saver.remember(&request(1, "a"));
        assert_eq!(saver.state(RequestId(1)), State::Saved);

        saver.update(&request(1, "b"));
        assert_eq!(saver.state(RequestId(1)), State::Dirty);
        saver.update(&request(1, "a"));
        assert_eq!(saver.state(RequestId(1)), State::Saved);
        assert!(!saver.has_pending());
    }

    #[test]
    fn writes_are_serialized_and_success_moves_the_baseline() {
        let mut saver = Saver::default();
        saver.remember(&request(1, "a"));
        saver.remember(&request(2, "x"));
        saver.update(&request(1, "b"));
        saver.update(&request(2, "y"));

        let (first, snapshot) = saver.next_write(None).unwrap();
        assert_eq!((first, snapshot.template.url.as_str()), (RequestId(1), "b"));
        assert_eq!(saver.state(RequestId(1)), State::Saving);
        assert!(saver.next_write(None).is_none(), "one write at a time");

        saver.finish(Ok(()));
        assert_eq!(saver.state(RequestId(1)), State::Saved);
        assert_eq!(saver.baseline(RequestId(1)).unwrap().template.url, "b");
        assert_eq!(saver.next_write(None).unwrap().0, RequestId(2));
    }

    #[test]
    fn an_edit_during_a_write_stays_pending_even_if_it_matches_the_old_baseline() {
        let mut saver = Saver::default();
        saver.remember(&request(1, "a"));
        saver.update(&request(1, "b"));
        saver.next_write(None).unwrap();

        saver.update(&request(1, "a"));
        saver.finish(Ok(()));

        assert_eq!(saver.state(RequestId(1)), State::Dirty);
        assert_eq!(saver.pending(RequestId(1)).unwrap().template.url, "a");
    }

    #[test]
    fn failed_writes_keep_their_snapshot_for_retry() {
        let mut saver = Saver::default();
        saver.remember(&request(1, "a"));
        saver.update(&request(1, "b"));
        saver.next_write(None).unwrap();

        saver.finish(Err("disk full".into()));

        assert_eq!(saver.state(RequestId(1)), State::Failed);
        assert_eq!(saver.failure(RequestId(1)), Some("disk full"));
        assert_eq!(saver.failures().count(), 1);
        let (_, retry) = saver.next_write(Some(RequestId(1))).unwrap();
        assert_eq!(retry.template.url, "b");
        saver.finish(Ok(()));
        assert_eq!(saver.state(RequestId(1)), State::Saved);
    }

    #[test]
    fn discarding_drops_edits_and_failures() {
        let mut saver = Saver::default();
        saver.update(&request(1, "b"));
        saver.update(&request(2, "c"));
        saver.discard(Some(RequestId(1)));
        assert_eq!(saver.state(RequestId(1)), State::Saved);
        assert!(saver.has_pending());
        saver.discard(None);
        assert!(!saver.has_pending());
    }

    #[test]
    fn tree_metadata_reaches_saved_and_pending_snapshots() {
        let mut saver = Saver::default();
        saver.remember(&request(1, "a"));
        saver.update(&request(1, "b"));

        saver.adopt_metadata(RequestId(1), |r| r.template.name = "renamed".into());

        assert_eq!(
            saver.baseline(RequestId(1)).unwrap().template.name,
            "renamed"
        );
        assert_eq!(
            saver.pending(RequestId(1)).unwrap().template.name,
            "renamed"
        );
    }

    #[test]
    fn remembering_never_overwrites_a_pending_edit_and_drafts_are_ignored() {
        let mut saver = Saver::default();
        saver.update(&request(1, "edited"));
        saver.remember(&request(1, "stale"));
        assert_eq!(saver.baseline(RequestId(1)), None);
        saver.update(&Request::default());
        assert_eq!(saver.next_write(Some(RequestId(9))), None);
        saver.forget(RequestId(1));
        assert!(!saver.has_pending());
        saver.finish(Ok(()));
    }
}
