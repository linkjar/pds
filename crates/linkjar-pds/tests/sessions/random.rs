//! Pauses credential issuance at the existing simulation RNG boundary.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

#[derive(Default)]
struct State {
    remaining: usize,
    released: bool,
    entered: Option<tokio::sync::mpsc::UnboundedSender<()>>,
}

#[derive(Default)]
pub struct Random {
    next: AtomicU64,
    state: Mutex<State>,
    release: Condvar,
}

impl Random {
    pub(super) fn next(&self) -> u64 {
        tokio::task::block_in_place(|| {
            let mut state = self.state.lock().unwrap();
            if state.remaining > 0 {
                state.remaining -= 1;
                state.entered.as_ref().unwrap().send(()).unwrap();
                while !state.released {
                    state = self.release.wait(state).unwrap();
                }
            }
            drop(state);
        });
        self.next.fetch_add(1, Ordering::Relaxed)
    }

    pub(super) fn pause(self: &Arc<Self>, count: usize) -> Pause {
        let (entered, receiver) = tokio::sync::mpsc::unbounded_channel();
        *self.state.lock().unwrap() = State {
            remaining: count,
            released: false,
            entered: Some(entered),
        };
        Pause {
            random: Arc::clone(self),
            receiver,
            count,
        }
    }
}

pub struct Pause {
    random: Arc<Random>,
    receiver: tokio::sync::mpsc::UnboundedReceiver<()>,
    count: usize,
}

impl Pause {
    pub(super) async fn entered(&mut self) {
        for _ in 0..self.count {
            tokio::time::timeout(Duration::from_secs(10), self.receiver.recv())
                .await
                .expect("credential issuance did not reach the RNG boundary")
                .expect("the RNG boundary closed");
        }
    }
}

impl Drop for Pause {
    fn drop(&mut self) {
        self.random.state.lock().unwrap().released = true;
        self.random.release.notify_all();
    }
}
