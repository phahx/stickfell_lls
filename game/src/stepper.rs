//! The world, stepped on a thread of its own, so a slow step (a yard full of
//! water) never holds up drawing: the screen keeps drawing at whatever rate
//! it runs at, smoothing between the last two states, while the next is
//! worked out.
//!
//! The thread owns the world (`W`). Each job is run on it in the order sent
//! and hands back a result (`R`, typically a copy of the world to draw);
//! results come back in order too. At most `MAX_IN_FLIGHT` jobs are out at
//! once: if the steps take longer than the ticks, the game slows down
//! rather than falling further and further behind.

use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Instant;

type Job<W, R> = Box<dyn FnOnce(&mut W) -> R + Send>;

/// How many jobs can be out at once (one being worked on, one waiting).
pub const MAX_IN_FLIGHT: usize = 2;

pub struct Stepper<W, R> {
    jobs: Option<mpsc::Sender<Job<W, R>>>,
    results: mpsc::Receiver<R>,
    in_flight: usize,
    thread: Option<JoinHandle<()>>,
}

impl<W: Send + 'static, R: Send + 'static> Stepper<W, R> {
    pub fn new(mut world: W) -> Self {
        let (jobs, inbox) = mpsc::channel::<Job<W, R>>();
        let (outbox, results) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("stickfell sim".into())
            .spawn(move || {
                // (until the game lets go of it)
                while let Ok(job) = inbox.recv() {
                    if outbox.send(job(&mut world)).is_err() {
                        break;
                    }
                }
            })
            .expect("the sim thread starts");
        Self { jobs: Some(jobs), results, in_flight: 0, thread: Some(thread) }
    }

    /// Whether there's room for another job.
    pub fn ready(&self) -> bool {
        self.in_flight < MAX_IN_FLIGHT
    }

    /// Sends a job (call `ready` first: past `MAX_IN_FLIGHT` it still goes,
    /// it just queues).
    pub fn send(&mut self, job: impl FnOnce(&mut W) -> R + Send + 'static) {
        if let Some(j) = &self.jobs {
            if j.send(Box::new(job)).is_ok() {
                self.in_flight += 1;
            }
        }
    }

    /// The results finished so far, oldest first (without waiting).
    pub fn finished(&mut self) -> Vec<R> {
        let out: Vec<R> = self.results.try_iter().collect();
        self.in_flight -= out.len().min(self.in_flight);
        out
    }

    /// Waits for every job sent so far, and returns their results.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn wait(&mut self) -> Vec<R> {
        let mut out = Vec::new();
        while self.in_flight > 0 {
            match self.results.recv() {
                Ok(r) => out.push(r),
                Err(_) => break,
            }
            self.in_flight -= 1;
        }
        out
    }
}

impl<W, R> Drop for Stepper<W, R> {
    fn drop(&mut self) {
        // (closing the job queue ends the thread once it's done what it's doing)
        self.jobs = None;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// How far to draw between the state before the newest and the newest: 0
/// when it's just come in, 1 a tick later (and held there until the next).
pub fn blend(arrived: Instant, tick_secs: f64) -> f32 {
    (arrived.elapsed().as_secs_f64() / tick_secs).clamp(0.0, 1.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_run_in_order_on_their_own_thread() {
        let main = std::thread::current().id();
        let mut s: Stepper<Vec<u32>, (usize, bool)> = Stepper::new(Vec::new());
        for k in 0..5u32 {
            s.send(move |v| {
                v.push(k);
                (v.len(), std::thread::current().id() != main)
            });
        }
        let got = s.wait();
        assert_eq!(got.iter().map(|r| r.0).collect::<Vec<_>>(), vec![1, 2, 3, 4, 5], "in order, each seeing the last");
        assert!(got.iter().all(|r| r.1), "not on the calling thread");
        assert!(s.ready() && s.finished().is_empty());
    }

    #[test]
    fn a_slow_job_doesnt_hold_up_the_caller() {
        let mut s: Stepper<(), ()> = Stepper::new(());
        let t = Instant::now();
        s.send(|_| std::thread::sleep(std::time::Duration::from_millis(50)));
        let sent = t.elapsed();
        assert!(sent.as_millis() < 10, "sending returns at once ({sent:?})");
        assert!(s.finished().is_empty(), "not done yet");
        assert_eq!(s.wait().len(), 1);
        assert!(t.elapsed().as_millis() >= 50);
    }
}
