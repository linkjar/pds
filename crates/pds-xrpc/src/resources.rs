//! Admission for work that holds threads, snapshots or large buffers.

use crate::error::XrpcError;
use axum::http::StatusCode;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Independent budgets keep slow transfers from occupying every worker.
#[derive(Debug)]
pub struct Resources {
    uploads: Arc<Semaphore>,
    imports: Arc<Semaphore>,
    exports: Arc<Semaphore>,
    subscriptions: Arc<Semaphore>,
    blocking: Arc<Semaphore>,
    audit: Arc<Semaphore>,
    repositories: Arc<Semaphore>,
    repository_queues: Mutex<HashMap<String, Weak<RepositoryQueue>>>,
}

#[derive(Debug)]
struct RepositoryQueue {
    slots: Arc<Semaphore>,
    writer: Arc<Semaphore>,
}

/// Moves into the worker so cancellation cannot admit a second writer.
pub(crate) struct RepositoryAdmission {
    _writer: OwnedSemaphorePermit,
    _slot: OwnedSemaphorePermit,
    _total: OwnedSemaphorePermit,
    // Keep the registry entry alive until all its permits have been released.
    _queue: Arc<RepositoryQueue>,
}

impl Default for Resources {
    fn default() -> Self {
        Self {
            uploads: Arc::new(Semaphore::new(8)),
            imports: Arc::new(Semaphore::new(2)),
            exports: Arc::new(Semaphore::new(8)),
            subscriptions: Arc::new(Semaphore::new(32)),
            blocking: Arc::new(Semaphore::new(64)),
            audit: Arc::new(Semaphore::new(4)),
            repositories: Arc::new(Semaphore::new(64)),
            repository_queues: Mutex::new(HashMap::new()),
        }
    }
}

fn acquire(budget: &Arc<Semaphore>) -> Result<OwnedSemaphorePermit, XrpcError> {
    Arc::clone(budget)
        .try_acquire_owned()
        .map_err(|_| XrpcError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            error: "ServerBusy".to_owned(),
            message: "The server is busy; retry this request later".to_owned(),
        })
}

impl Resources {
    /// At most 64 repository operations are active or waiting, eight per DID.
    /// Waiting takes no blocking worker. The execution permit is FIFO.
    pub(crate) async fn repository(&self, did: &str) -> Result<RepositoryAdmission, XrpcError> {
        let total = acquire(&self.repositories)?;
        let queue = {
            let mut queues = self
                .repository_queues
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(queue) = queues.get(did).and_then(Weak::upgrade) {
                queue
            } else {
                // Only admitted work owns a queue. Prune expired keys before
                // insertion, keeping the registry within the total budget.
                queues.retain(|_, queue| queue.strong_count() > 0);
                let queue = Arc::new(RepositoryQueue {
                    slots: Arc::new(Semaphore::new(8)),
                    writer: Arc::new(Semaphore::new(1)),
                });
                queues.insert(did.to_owned(), Arc::downgrade(&queue));
                queue
            }
        };
        let slot = acquire(&queue.slots)?;
        let writer = Arc::clone(&queue.writer)
            .acquire_owned()
            .await
            .map_err(XrpcError::internal)?;
        Ok(RepositoryAdmission {
            _writer: writer,
            _slot: slot,
            _total: total,
            _queue: queue,
        })
    }

    /// Held inside the upload's blocking writer until it closes.
    pub fn upload(&self) -> Result<OwnedSemaphorePermit, XrpcError> {
        acquire(&self.uploads)
    }
    /// Held while collecting and committing an import.
    pub fn import(&self) -> Result<OwnedSemaphorePermit, XrpcError> {
        acquire(&self.imports)
    }
    /// Held inside the response producer until its snapshot is released.
    pub fn export(&self) -> Result<OwnedSemaphorePermit, XrpcError> {
        acquire(&self.exports)
    }
    /// Held for the complete lifetime of a firehose connection.
    pub fn subscription(&self) -> Result<OwnedSemaphorePermit, XrpcError> {
        acquire(&self.subscriptions)
    }
    /// Held inside a blocking job, including after its caller disconnects.
    pub fn blocking(&self) -> Result<OwnedSemaphorePermit, XrpcError> {
        acquire(&self.blocking)
    }
    /// Audit writes wait asynchronously, so a busy server does not discard
    /// an already completed action's audit entry or occupy extra workers.
    pub async fn audit(&self) -> Result<OwnedSemaphorePermit, XrpcError> {
        Arc::clone(&self.audit)
            .acquire_owned()
            .await
            .map_err(XrpcError::internal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfers_have_independent_bounded_budgets_and_release_capacity() {
        let budgets = Resources::default();
        let mut uploads: Vec<_> = (0..8).map(|_| budgets.upload().unwrap()).collect();
        assert_eq!(uploads.len(), 8);
        assert_eq!(
            budgets.upload().unwrap_err().status,
            StatusCode::SERVICE_UNAVAILABLE
        );
        let _import = budgets.import().unwrap();
        let _export = budgets.export().unwrap();
        let _ordinary = budgets.blocking().unwrap();
        uploads.pop();
        assert!(budgets.upload().is_ok());
        let _second_import = budgets.import().unwrap();
        assert!(budgets.import().is_err());
    }
}
