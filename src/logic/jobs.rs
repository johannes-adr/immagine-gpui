//! Background jobs (for now: downloading a set of assets). Each job is stored on its own (as JSON) so it can be
//! paused, resumed and picked up again after a restart; the runner lives in `immagine/jobs.rs`.

use serde::{Deserialize, Serialize};

/// How many finished jobs are kept for the list; older ones are deleted.
pub const KEEP_FINISHED: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobState {
    /// Works (or waits for the runner); resumed automatically at start.
    Running,
    Paused,
    Done,
    Cancelled,
}

impl JobState {
    pub fn finished(self) -> bool {
        matches!(self, JobState::Done | JobState::Cancelled)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobItem {
    pub id: String,
    pub video: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: u64,
    pub state: JobState,
    pub items: Vec<JobItem>,
    /// Index of the next item to work on; everything before it is done, failed or skipped.
    pub next: usize,
    pub done: usize,
    pub failed: usize,
    /// Already stored before the job reached them.
    pub skipped: usize,
    /// Bytes received for finished items.
    pub bytes: u64,
    /// Time spent running, summed over every stretch (pauses and app restarts do not count).
    pub active_ms: f64,
    /// Order in which jobs finished (higher = later), for "the last few finished".
    pub finished_seq: Option<u64>,
    pub last_error: Option<String>,
}

impl Job {
    pub fn new(id: u64, items: Vec<JobItem>) -> Self {
        Self {
            id,
            state: JobState::Running,
            items,
            next: 0,
            done: 0,
            failed: 0,
            skipped: 0,
            bytes: 0,
            active_ms: 0.,
            finished_seq: None,
            last_error: None,
        }
    }

    pub fn key(&self) -> String {
        self.id.to_string()
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }

    pub fn total(&self) -> usize {
        self.items.len()
    }

    /// Items handled so far (done, failed or skipped).
    pub fn handled(&self) -> usize {
        self.next.min(self.items.len())
    }

    pub fn current(&self) -> Option<&JobItem> {
        self.items.get(self.next)
    }

    /// Records the outcome of the current item and moves on; finishes the job after the last one.
    /// `bytes` = `Some(n)` stored n bytes, `None` failed (with `error`). Returns true when the job is now done.
    pub fn record(&mut self, outcome: Outcome, finish_seq: u64) -> bool {
        match outcome {
            Outcome::Stored(n) => {
                self.done += 1;
                self.bytes += n;
            }
            Outcome::Skipped => self.skipped += 1,
            Outcome::Failed(e) => {
                self.failed += 1;
                self.last_error = Some(e);
            }
        }
        self.next += 1;
        if self.next >= self.items.len() {
            self.finish(JobState::Done, finish_seq);
            return true;
        }
        false
    }

    pub fn finish(&mut self, state: JobState, seq: u64) {
        self.state = state;
        self.finished_seq = Some(seq);
    }

    /// Average speed in bytes per second over the running time.
    pub fn rate(&self) -> Option<f64> {
        (self.active_ms > 500. && self.bytes > 0).then(|| self.bytes as f64 / (self.active_ms / 1000.))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Stored(u64),
    Skipped,
    Failed(String),
}

/// Ids for a new job and the next finish order, from the jobs already there.
pub fn next_ids(jobs: &[Job]) -> (u64, u64) {
    let id = jobs.iter().map(|j| j.id).max().map_or(1, |m| m + 1);
    let seq = jobs.iter().filter_map(|j| j.finished_seq).max().map_or(1, |m| m + 1);
    (id, seq)
}

/// The job the runner should work on: the oldest running one.
pub fn runnable(jobs: &[Job]) -> Option<u64> {
    jobs.iter().filter(|j| j.state == JobState::Running).map(|j| j.id).min()
}

/// Unfinished jobs (oldest first), then the last `KEEP_FINISHED` finished ones (latest first).
pub fn listed(jobs: &[Job]) -> (Vec<&Job>, Vec<&Job>) {
    let mut active: Vec<&Job> = jobs.iter().filter(|j| !j.state.finished()).collect();
    active.sort_by_key(|j| j.id);
    let mut finished: Vec<&Job> = jobs.iter().filter(|j| j.state.finished()).collect();
    finished.sort_by_key(|j| std::cmp::Reverse(j.finished_seq.unwrap_or(0)));
    finished.truncate(KEEP_FINISHED);
    (active, finished)
}

/// Removes finished jobs beyond the last `KEEP_FINISHED`; returns their ids (to delete from storage).
pub fn prune(jobs: &mut Vec<Job>) -> Vec<u64> {
    let keep: Vec<u64> = listed(jobs).1.iter().map(|j| j.id).collect();
    let mut gone = Vec::new();
    jobs.retain(|j| {
        let drop = j.state.finished() && !keep.contains(&j.id);
        if drop {
            gone.push(j.id);
        }
        !drop
    });
    gone
}

/// "1:05 min", "42 s", "1:02 h".
pub fn duration_text(ms: f64) -> String {
    let s = (ms / 1000.).max(0.) as u64;
    if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{}:{:02} min", s / 60, s % 60)
    } else {
        format!("{}:{:02} h", s / 3600, (s / 60) % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(n: usize) -> Vec<JobItem> {
        (0..n).map(|i| JobItem { id: format!("a{i}"), video: i % 2 == 1 }).collect()
    }

    #[test]
    fn record_counts_outcomes_and_finishes_after_the_last_item() {
        let mut job = Job::new(1, items(3));
        assert!(!job.record(Outcome::Stored(100), 1));
        assert!(!job.record(Outcome::Skipped, 1));
        assert_eq!(job.current().unwrap().id, "a2");
        assert!(job.record(Outcome::Failed("404".into()), 7));
        assert_eq!((job.done, job.skipped, job.failed, job.bytes, job.handled()), (1, 1, 1, 100, 3));
        assert_eq!(job.state, JobState::Done);
        assert_eq!(job.finished_seq, Some(7));
        assert_eq!(job.last_error.as_deref(), Some("404"));
    }

    #[test]
    fn round_trips_through_json() {
        let mut job = Job::new(4, items(2));
        job.state = JobState::Paused;
        job.active_ms = 1234.;
        assert_eq!(Job::from_bytes(&job.to_bytes()), Some(job));
        assert_eq!(Job::from_bytes(b"nonsense"), None);
    }

    #[test]
    fn runnable_is_the_oldest_running_job() {
        let mut a = Job::new(2, items(1));
        let b = Job::new(5, items(1));
        let mut c = Job::new(1, items(1));
        c.state = JobState::Paused;
        assert_eq!(runnable(&[a.clone(), b.clone(), c.clone()]), Some(2));
        a.state = JobState::Done;
        assert_eq!(runnable(&[a, b, c]), Some(5));
    }

    #[test]
    fn keeps_the_last_three_finished_jobs() {
        let mut jobs: Vec<Job> = (1..=6)
            .map(|i| {
                let mut j = Job::new(i, items(1));
                if i <= 5 {
                    j.finish(JobState::Done, i);
                }
                j
            })
            .collect();
        let (active, finished) = listed(&jobs);
        assert_eq!(active.iter().map(|j| j.id).collect::<Vec<_>>(), vec![6]);
        assert_eq!(finished.iter().map(|j| j.id).collect::<Vec<_>>(), vec![5, 4, 3]);
        let mut gone = prune(&mut jobs);
        gone.sort();
        assert_eq!(gone, vec![1, 2]);
        assert_eq!(jobs.len(), 4);
        assert_eq!(next_ids(&jobs), (7, 6));
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(duration_text(42_400.), "42 s");
        assert_eq!(duration_text(65_000.), "1:05 min");
        assert_eq!(duration_text(3_720_000.), "1:02 h");
    }
}
