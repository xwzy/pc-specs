//! Rule actions share a bounded queue and four workers. A burst of rules or
//! repeated test clicks must not create an unbounded number of OS threads.

use std::sync::{Arc, Mutex, OnceLock, mpsc};

type Job = Box<dyn FnOnce() + Send + 'static>;

struct Queue(mpsc::SyncSender<Job>);

impl Queue {
    fn new(workers: usize, capacity: usize) -> Option<Self> {
        let (tx, rx) = mpsc::sync_channel::<Job>(capacity);
        let rx = Arc::new(Mutex::new(rx));
        let mut started = 0;
        for index in 0..workers {
            let rx = rx.clone();
            if crate::state::spawn_thread(&format!("pc-specs-action-{index}"), move || {
                loop {
                    let job = rx.lock().unwrap_or_else(|e| e.into_inner()).recv();
                    let Ok(job) = job else { break };
                    pc_specs_core::util::guarded("background action", job);
                }
            }) {
                started += 1;
            }
        }
        (started > 0).then_some(Self(tx))
    }

    fn submit(&self, job: impl FnOnce() + Send + 'static) -> bool {
        self.0.try_send(Box::new(job)).is_ok()
    }
}

pub fn submit(job: impl FnOnce() + Send + 'static) -> bool {
    static QUEUE: OnceLock<Option<Queue>> = OnceLock::new();
    QUEUE
        .get_or_init(|| Queue::new(4, 128))
        .as_ref()
        .is_some_and(|queue| queue.submit(job))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_blocked_action_bounds_the_queue_and_panics_do_not_kill_workers() {
        let queue = Queue::new(1, 2).unwrap();
        let (started, ready) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        assert!(queue.submit(move || {
            started.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
        }));
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(queue.submit(|| panic!("injected action failure")));
        let (done, finished) = mpsc::channel();
        assert!(queue.submit(move || {
            done.send(()).unwrap();
        }));
        for _ in 0..10_000 {
            assert!(!queue.submit(|| {}));
        }
        release.send(()).unwrap();
        finished.recv_timeout(Duration::from_secs(2)).unwrap();
    }
}
