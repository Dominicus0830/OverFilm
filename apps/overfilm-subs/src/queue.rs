//! Several files in a row: the window keeps a queue and makes their captions one after another
//! with the same settings.

use std::path::{Path, PathBuf};

/// Where a queued file is.
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Waiting,
    Running,
    Done {
        srt: PathBuf,
        captions: usize,
        seconds_taken: u64,
    },
    Failed(String),
    /// Cancelled, or skipped by the user.
    Skipped,
}

impl State {
    /// Finished one way or another.
    pub fn ended(&self) -> bool {
        matches!(self, State::Done { .. } | State::Failed(_) | State::Skipped)
    }
}

/// One file in the queue.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub input: PathBuf,
    /// The audio track to recognise (0 = the first).
    pub audio_track: usize,
    /// Skip long quiet stretches (`None`: follow the sound source).
    pub skip_silence: Option<bool>,
    /// SRT to write (`None`: beside the input).
    pub output: Option<PathBuf>,
    pub state: State,
}

impl Item {
    pub fn new(input: PathBuf) -> Self {
        Self { input, audio_track: 0, skip_silence: None, output: None, state: State::Waiting }
    }

    pub fn name(&self) -> String {
        self.input.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

/// Files queued at most (each run takes minutes; this is a whole night's worth and more).
pub const MAX_ITEMS: usize = 200;

/// The files of this batch, in order.
#[derive(Clone, Debug, Default)]
pub struct Queue {
    pub items: Vec<Item>,
}

impl Queue {
    /// Start over with one file.
    pub fn reset(&mut self, input: PathBuf) {
        self.items = vec![Item::new(input)];
    }

    /// Add files to the end, leaving out ones already waiting or running and anything past
    /// [`MAX_ITEMS`]. Returns how many were added.
    pub fn add(&mut self, paths: impl IntoIterator<Item = PathBuf>) -> usize {
        let mut added = 0;
        for p in paths {
            if self.items.len() >= MAX_ITEMS {
                break;
            }
            if self.items.iter().any(|i| i.input == p && !i.state.ended()) {
                continue;
            }
            self.items.push(Item::new(p));
            added += 1;
        }
        added
    }

    /// Take a file out (not the one running).
    pub fn remove(&mut self, k: usize) -> bool {
        if self.items.get(k).is_some_and(|i| i.state != State::Running) {
            self.items.remove(k);
            true
        } else {
            false
        }
    }

    /// The next file to start.
    pub fn next_waiting(&self) -> Option<usize> {
        self.items.iter().position(|i| i.state == State::Waiting)
    }

    pub fn running(&self) -> Option<usize> {
        self.items.iter().position(|i| i.state == State::Running)
    }

    /// More than one file: results are listed instead of opened one by one.
    pub fn is_batch(&self) -> bool {
        self.items.len() > 1
    }

    /// Nothing waiting or running.
    pub fn finished(&self) -> bool {
        self.items.iter().all(|i| i.state.ended())
    }

    /// Mark every waiting file skipped (the user cancelled the lot).
    pub fn skip_waiting(&mut self) {
        for i in &mut self.items {
            if i.state == State::Waiting {
                i.state = State::Skipped;
            }
        }
    }

    /// (done, failed, skipped) so far.
    pub fn counts(&self) -> (usize, usize, usize) {
        self.items.iter().fold((0, 0, 0), |(d, f, s), i| match i.state {
            State::Done { .. } => (d + 1, f, s),
            State::Failed(_) => (d, f + 1, s),
            State::Skipped => (d, f, s + 1),
            _ => (d, f, s),
        })
    }

    /// Progress of the whole batch, given the running file's own progress (0–1).
    pub fn fraction(&self, current: f32) -> f32 {
        let n = self.items.len();
        if n == 0 {
            return 0.0;
        }
        let ended = self.items.iter().filter(|i| i.state.ended()).count() as f32;
        let now = if self.running().is_some() { current.clamp(0.0, 1.0) } else { 0.0 };
        ((ended + now) / n as f32).clamp(0.0, 1.0)
    }

    /// The position (1-based) of the running file, for "2/5".
    pub fn position(&self) -> Option<(usize, usize)> {
        self.running().map(|k| (k + 1, self.items.len()))
    }

    /// Whether `p` is in the queue and not finished.
    pub fn holds(&self, p: &Path) -> bool {
        self.items.iter().any(|i| i.input == p && !i.state.ended())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(names: &[&str]) -> Queue {
        let mut q = Queue::default();
        q.add(names.iter().map(PathBuf::from));
        q
    }

    #[test]
    fn files_run_in_order_and_duplicates_are_left_out() {
        let mut q = q(&["a.mp4", "b.mp4"]);
        assert_eq!(q.add([PathBuf::from("a.mp4"), PathBuf::from("c.mp4")]), 1);
        assert!(q.is_batch());
        assert_eq!(q.next_waiting(), Some(0));
        if let Some(i) = q.items.get_mut(0) {
            i.state = State::Running;
        }
        assert_eq!(q.position(), Some((1, 3)));
        assert!(!q.remove(0), "the running file stays");
        assert!(q.remove(2));
        assert_eq!(q.fraction(0.5), 0.25);
        if let Some(i) = q.items.get_mut(0) {
            i.state = State::Done { srt: "a.srt".into(), captions: 3, seconds_taken: 9 };
        }
        // a finished file can be queued again
        assert_eq!(q.add([PathBuf::from("a.mp4")]), 1);
        assert_eq!(q.next_waiting(), Some(1));
        assert!(q.holds(Path::new("a.mp4")));
        assert!(!q.finished());
        q.skip_waiting();
        assert!(q.finished());
        assert_eq!(q.counts(), (1, 0, 2));
        assert_eq!(q.fraction(0.9), 1.0);
    }

    #[test]
    fn the_queue_is_capped() {
        let mut q = Queue::default();
        let added = q.add((0..MAX_ITEMS + 50).map(|k| PathBuf::from(format!("{k}.mp4"))));
        assert_eq!(added, MAX_ITEMS);
        assert_eq!(Queue::default().fraction(0.5), 0.0);
        q.reset("x.mp4".into());
        assert!(!q.is_batch());
    }
}
