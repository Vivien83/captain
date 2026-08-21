//! Cooperative shutdown shared by foreground and native-service Node runners.

use std::fmt;
use tokio::sync::watch;

#[derive(Clone)]
pub struct NodeShutdown {
    receiver: watch::Receiver<bool>,
}

#[derive(Clone)]
pub struct NodeShutdownHandle {
    sender: watch::Sender<bool>,
}

pub fn node_shutdown_channel() -> (NodeShutdownHandle, NodeShutdown) {
    let (sender, receiver) = watch::channel(false);
    (NodeShutdownHandle { sender }, NodeShutdown { receiver })
}

impl NodeShutdownHandle {
    pub fn cancel(&self) {
        self.sender.send_replace(true);
    }
}

impl NodeShutdown {
    pub fn is_cancelled(&self) -> bool {
        *self.receiver.borrow()
    }

    pub(crate) async fn wait(&mut self) {
        if self.is_cancelled() {
            return;
        }
        while self.receiver.changed().await.is_ok() {
            if self.is_cancelled() {
                return;
            }
        }
    }
}

impl fmt::Debug for NodeShutdown {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeShutdown")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

impl fmt::Debug for NodeShutdownHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NodeShutdownHandle")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancellation_is_idempotent_and_visible_to_existing_clones() {
        let (handle, mut shutdown) = node_shutdown_channel();
        let mut clone = shutdown.clone();
        assert!(!shutdown.is_cancelled());

        handle.cancel();
        handle.cancel();
        shutdown.wait().await;
        clone.wait().await;

        assert!(shutdown.is_cancelled());
        assert!(clone.is_cancelled());
    }
}
