//! Standard channels whose sends also wake an async consumer, so the engine and UI react
//! to input, events and frames as they arrive instead of polling on a timer.
use std::sync::{
    Arc,
    mpsc::{self, Receiver, SendError},
};

use tokio::sync::Notify;

pub struct Sender<T> {
    inner: mpsc::Sender<T>,
    wake: Arc<Notify>,
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            wake: self.wake.clone(),
        }
    }
}

impl<T> Sender<T> {
    pub fn send(&self, value: T) -> Result<(), SendError<T>> {
        let sent = self.inner.send(value);
        self.wake.notify_one();
        sent
    }

    /// Wake the consumer without a message, after updating state it reads (a frame slot).
    pub fn wake(&self) {
        self.wake.notify_one();
    }
}

/// A channel whose consumer waits on `wake`.
pub fn channel<T>(wake: &Arc<Notify>) -> (Sender<T>, Receiver<T>) {
    let (inner, receiver) = mpsc::channel();
    (
        Sender {
            inner,
            wake: wake.clone(),
        },
        receiver,
    )
}

/// A channel nobody waits on (tests, and consumers that poll).
#[cfg(test)]
pub fn detached<T>() -> (Sender<T>, Receiver<T>) {
    channel(&Arc::new(Notify::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sending_and_waking_both_release_a_waiter() {
        let wake = Arc::new(Notify::new());
        let (sender, receiver) = channel(&wake);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            sender.send(1).unwrap();
            // The permit is stored even though nobody was waiting yet.
            tokio::time::timeout(std::time::Duration::from_millis(50), wake.notified())
                .await
                .expect("send did not wake");
            sender.wake();
            tokio::time::timeout(std::time::Duration::from_millis(50), wake.notified())
                .await
                .expect("wake did not wake");
        });
        assert_eq!(receiver.try_recv(), Ok(1));
    }
}
