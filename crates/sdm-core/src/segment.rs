//! Splitting a file into segments and handing them out to connections.
//!
//! Every segment is a byte range `[start, end)` that one connection fills from
//! front to back. When a connection finishes its own segment it steals the
//! back half of the segment with the most bytes left, so all connections stay
//! busy until the very end (the trick that makes download accelerators fast).

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::state::SavedSegment;

/// Locks a mutex, ignoring poisoning: our critical sections never leave the
/// data in an inconsistent state.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Positions inside one segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SegmentState {
    /// Where the segment started; bytes before it belong to other segments.
    pub start: u64,
    /// Bytes up to here were received and are written or about to be.
    pub claimed: u64,
    /// Bytes up to here are safely written to the file.
    pub written: u64,
    /// One past the last byte. `u64::MAX` when the size is unknown.
    pub end: u64,
    /// Whether a connection is working on this segment.
    pub assigned: bool,
}

#[derive(Debug)]
pub(crate) struct Segment(Mutex<SegmentState>);

impl Segment {
    fn new(start: u64, pos: u64, end: u64, assigned: bool) -> Self {
        Self(Mutex::new(SegmentState {
            start,
            claimed: pos,
            written: pos,
            end,
            assigned,
        }))
    }

    pub(crate) fn state(&self) -> SegmentState {
        *lock(&self.0)
    }

    pub(crate) fn written(&self) -> u64 {
        lock(&self.0).written
    }

    /// Prepares a new request: forgets bytes that were received but never
    /// written and returns the range still to fetch, or `None` when done.
    pub(crate) fn begin(&self) -> Option<(u64, u64)> {
        let mut s = lock(&self.0);
        s.claimed = s.written;
        (s.written < s.end).then_some((s.written, s.end))
    }

    /// Reserves up to `len` bytes that just arrived and returns how many of
    /// them belong to this segment (the end may have moved because another
    /// connection took over the back part).
    pub(crate) fn claim(&self, len: u64) -> u64 {
        let mut s = lock(&self.0);
        let take = len.min(s.end.saturating_sub(s.claimed));
        s.claimed += take;
        take
    }

    /// Records that `len` claimed bytes reached the file.
    pub(crate) fn commit(&self, len: u64) {
        let mut s = lock(&self.0);
        s.written += len;
        debug_assert!(s.written <= s.claimed);
    }

    /// Whether every byte of the segment has been claimed.
    pub(crate) fn is_claimed(&self) -> bool {
        let s = lock(&self.0);
        s.claimed >= s.end
    }

    pub(crate) fn is_unbounded(&self) -> bool {
        lock(&self.0).end == u64::MAX
    }

    /// Ends a segment of unknown size at the current position.
    pub(crate) fn close(&self) {
        let mut s = lock(&self.0);
        s.end = s.claimed;
    }

    /// Throws away all progress; used when the server cannot resume.
    pub(crate) fn restart(&self) {
        let mut s = lock(&self.0);
        s.claimed = s.start;
        s.written = s.start;
    }
}

#[derive(Debug)]
pub(crate) struct Scheduler {
    inner: Mutex<Inner>,
    splittable: bool,
    min_split: u64,
}

#[derive(Debug)]
struct Inner {
    segments: Vec<Arc<Segment>>,
    /// Connections that are still running.
    workers: usize,
}

impl Scheduler {
    pub(crate) fn new(
        plan: &[SavedSegment],
        splittable: bool,
        min_split: u64,
        workers: usize,
    ) -> Self {
        let segments = plan
            .iter()
            .map(|s| Arc::new(Segment::new(s.start, s.pos, s.end, false)))
            .collect();
        Self {
            inner: Mutex::new(Inner { segments, workers }),
            splittable,
            min_split: min_split.max(1),
        }
    }

    /// Gives a connection its next piece of work, or `None` when there is
    /// nothing left worth doing (the connection then stops).
    pub(crate) fn next(&self) -> Option<Arc<Segment>> {
        let mut inner = lock(&self.inner);

        for segment in &inner.segments {
            let mut s = lock(&segment.0);
            if !s.assigned && s.claimed < s.end {
                s.assigned = true;
                return Some(segment.clone());
            }
        }

        if self.splittable
            && let Some(segment) = self.split_largest(&mut inner)
        {
            return Some(segment);
        }

        inner.workers = inner.workers.saturating_sub(1);
        None
    }

    fn split_largest(&self, inner: &mut Inner) -> Option<Arc<Segment>> {
        let victim = inner
            .segments
            .iter()
            .filter_map(|segment| {
                let s = segment.state();
                (s.assigned && s.end != u64::MAX)
                    .then(|| (s.end.saturating_sub(s.claimed), segment))
            })
            .max_by_key(|(remaining, _)| *remaining)
            .map(|(_, segment)| segment.clone())?;

        let mut s = lock(&victim.0);
        let remaining = s.end.saturating_sub(s.claimed);
        if remaining < 2 * self.min_split {
            return None;
        }
        let middle = s.claimed + remaining / 2;
        let stolen = Arc::new(Segment::new(middle, middle, s.end, true));
        s.end = middle;
        drop(s);
        inner.segments.push(stolen.clone());
        Some(stolen)
    }

    /// Lets a connection that keeps failing stop and hand its segment to
    /// the others. The last connection is never allowed to retire.
    pub(crate) fn try_retire(&self, segment: &Segment) -> bool {
        let mut inner = lock(&self.inner);
        if inner.workers <= 1 {
            return false;
        }
        lock(&segment.0).assigned = false;
        inner.workers -= 1;
        true
    }

    /// Called by a connection that stops because of an error or a pause.
    pub(crate) fn leave(&self) {
        let mut inner = lock(&self.inner);
        inner.workers = inner.workers.saturating_sub(1);
    }

    /// All segments ordered by position.
    pub(crate) fn snapshot(&self) -> Vec<SegmentState> {
        let mut states: Vec<_> = lock(&self.inner)
            .segments
            .iter()
            .map(|s| s.state())
            .collect();
        states.sort_by_key(|s| s.start);
        states
    }

    /// Bytes received so far, including earlier sessions.
    pub(crate) fn downloaded(&self) -> u64 {
        self.snapshot().iter().map(|s| s.claimed - s.start).sum()
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.snapshot().iter().all(|s| s.written >= s.end)
    }

    /// Progress that is safe to persist: only bytes already on disk count.
    pub(crate) fn saved_segments(&self) -> Vec<SavedSegment> {
        self.snapshot()
            .iter()
            .map(|s| SavedSegment {
                start: s.start,
                pos: s.written,
                end: s.end,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(ranges: &[(u64, u64)]) -> Vec<SavedSegment> {
        ranges
            .iter()
            .map(|&(start, end)| SavedSegment {
                start,
                pos: start,
                end,
            })
            .collect()
    }

    #[test]
    fn hands_out_planned_segments_first() {
        let scheduler = Scheduler::new(&plan(&[(0, 100), (100, 200)]), true, 10, 3);
        assert_eq!(scheduler.next().unwrap().state().start, 0);
        assert_eq!(scheduler.next().unwrap().state().start, 100);
    }

    #[test]
    fn steals_back_half_of_largest_segment() {
        let scheduler = Scheduler::new(&plan(&[(0, 100), (100, 300)]), true, 10, 3);
        let first = scheduler.next().unwrap();
        let second = scheduler.next().unwrap();
        assert_eq!(second.claim(40), 40);

        let stolen = scheduler.next().unwrap().state();
        // Segment 100..300 has 160 bytes left after position 140.
        assert_eq!((stolen.start, stolen.end), (220, 300));
        assert_eq!(second.state().end, 220);
        assert_eq!(first.state().end, 100);
    }

    #[test]
    fn claims_stop_at_moved_end() {
        let scheduler = Scheduler::new(&plan(&[(0, 100)]), true, 10, 2);
        let owner = scheduler.next().unwrap();
        assert_eq!(owner.claim(10), 10);
        let stolen = scheduler.next().unwrap();
        assert_eq!(stolen.state().start, 55);
        assert_eq!(owner.claim(100), 45);
        assert!(owner.is_claimed());
    }

    #[test]
    fn does_not_split_small_segments() {
        let scheduler = Scheduler::new(&plan(&[(0, 15)]), true, 10, 2);
        let _owner = scheduler.next().unwrap();
        assert!(scheduler.next().is_none());
    }

    #[test]
    fn does_not_split_when_not_splittable() {
        let scheduler = Scheduler::new(&plan(&[(0, 1000)]), false, 10, 2);
        let _owner = scheduler.next().unwrap();
        assert!(scheduler.next().is_none());
    }

    #[test]
    fn begin_forgets_unwritten_bytes() {
        let scheduler = Scheduler::new(&plan(&[(0, 100)]), false, 10, 1);
        let segment = scheduler.next().unwrap();
        assert_eq!(segment.begin(), Some((0, 100)));
        segment.claim(30);
        segment.commit(20);
        assert_eq!(segment.begin(), Some((20, 100)));
        segment.claim(80);
        segment.commit(80);
        assert_eq!(segment.begin(), None);
        assert!(scheduler.is_complete());
        assert_eq!(scheduler.downloaded(), 100);
    }

    #[test]
    fn retired_segments_are_picked_up_again() {
        let scheduler = Scheduler::new(&plan(&[(0, 15), (15, 30)]), true, 10, 2);
        let a = scheduler.next().unwrap();
        let _b = scheduler.next().unwrap();
        assert!(scheduler.try_retire(&a));
        // The last connection must keep going.
        assert!(!scheduler.try_retire(&a));
        assert_eq!(scheduler.next().unwrap().state().start, 0);
    }

    #[test]
    fn resumes_from_saved_positions() {
        let saved = [
            SavedSegment {
                start: 0,
                pos: 50,
                end: 50,
            },
            SavedSegment {
                start: 50,
                pos: 70,
                end: 100,
            },
        ];
        let scheduler = Scheduler::new(&saved, true, 10, 2);
        assert_eq!(scheduler.downloaded(), 70);
        let segment = scheduler.next().unwrap();
        assert_eq!(segment.begin(), Some((70, 100)));
    }
}
