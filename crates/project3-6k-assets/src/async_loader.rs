//! Bounded background CPU model loading with back-pressure at both queue ends.

use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, SyncSender, TrySendError},
    thread::{self, JoinHandle},
    time::Duration,
};

use thiserror::Error;

use crate::{CpuModelAsset, ModelImportError, ModelImportLimits, import_gltf};

/// Identity for one model request submitted to [`AsyncModelLoader`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ModelLoadId(u64);

impl ModelLoadId {
    /// Returns the monotonically assigned request number.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Completion data returned after CPU import succeeds or fails.
#[derive(Debug)]
pub struct ModelLoadEvent {
    /// Request identity returned by [`AsyncModelLoader::request`].
    pub id: ModelLoadId,
    /// Source model path.
    pub path: PathBuf,
    /// Imported CPU asset or a validation/decoding error.
    pub result: Result<CpuModelAsset, ModelImportError>,
}

/// Errors from submitting a job to the bounded CPU loading queue.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum AsyncLoadError {
    /// The request queue is full; the caller should retry after draining completions.
    #[error("CPU asset loading queue is full")]
    QueueFull,
    /// The worker has stopped or the request identity space is exhausted.
    #[error("CPU asset loading worker is stopped")]
    Stopped,
}

struct LoadRequest {
    id: ModelLoadId,
    path: PathBuf,
}

/// One CPU worker that imports glTF/GLB off the render thread.
///
/// Both queued work and completed-but-uncollected models are bounded. When the completion queue
/// fills, the worker blocks and the request queue naturally applies back-pressure to producers.
pub struct AsyncModelLoader {
    requests: Option<SyncSender<LoadRequest>>,
    completed: Option<Receiver<ModelLoadEvent>>,
    worker: Option<JoinHandle<()>>,
    next_id: u64,
}

impl std::fmt::Debug for AsyncModelLoader {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AsyncModelLoader")
            .field("next_id", &self.next_id)
            .field("worker_running", &self.worker.is_some())
            .finish()
    }
}

impl AsyncModelLoader {
    /// Starts one worker with a bounded request and completion queue.
    pub fn new(limits: ModelImportLimits, queue_capacity: usize) -> std::io::Result<Self> {
        let capacity = queue_capacity.max(1);
        let (request_tx, request_rx) = mpsc::sync_channel::<LoadRequest>(capacity);
        let (completed_tx, completed_rx) = mpsc::sync_channel::<ModelLoadEvent>(capacity);
        let worker = thread::Builder::new()
            .name("project3-6k-asset-loader".to_owned())
            .spawn(move || {
                while let Ok(request) = request_rx.recv() {
                    let event = ModelLoadEvent {
                        id: request.id,
                        path: request.path.clone(),
                        result: import_gltf(&request.path, limits),
                    };
                    if completed_tx.send(event).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            requests: Some(request_tx),
            completed: Some(completed_rx),
            worker: Some(worker),
            next_id: 0,
        })
    }

    /// Queues a glTF or GLB import without blocking the calling/render thread.
    pub fn request(&mut self, path: impl Into<PathBuf>) -> Result<ModelLoadId, AsyncLoadError> {
        let id = ModelLoadId(self.next_id);
        let next_id = self.next_id.checked_add(1).ok_or(AsyncLoadError::Stopped)?;
        let request = LoadRequest {
            id,
            path: path.into(),
        };
        let Some(requests) = &self.requests else {
            return Err(AsyncLoadError::Stopped);
        };
        match requests.try_send(request) {
            Ok(()) => {
                self.next_id = next_id;
                Ok(id)
            }
            Err(TrySendError::Full(_)) => Err(AsyncLoadError::QueueFull),
            Err(TrySendError::Disconnected(_)) => Err(AsyncLoadError::Stopped),
        }
    }

    /// Returns all completed requests currently available without blocking.
    pub fn poll(&self) -> Vec<ModelLoadEvent> {
        let Some(completed) = &self.completed else {
            return Vec::new();
        };
        let mut events = Vec::new();
        while let Ok(event) = completed.try_recv() {
            events.push(event);
        }
        events
    }

    /// Waits for one completion for at most `timeout`.
    pub fn wait_next(&self, timeout: Duration) -> Option<ModelLoadEvent> {
        self.completed.as_ref()?.recv_timeout(timeout).ok()
    }
}

impl Drop for AsyncModelLoader {
    fn drop(&mut self) {
        // Close the result end first so an in-progress worker exits as soon as it finishes its
        // current CPU import instead of blocking forever on an unconsumed completion.
        self.completed.take();
        self.requests.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    use super::AsyncModelLoader;
    use crate::ModelImportLimits;

    static NEXT_TEMP_ID: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn worker_reports_io_failures_as_completion_events() {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let missing_path = std::env::temp_dir().join(format!(
            "project36k-missing-model-{}-{id}.gltf",
            std::process::id()
        ));
        let mut loader = AsyncModelLoader::new(ModelImportLimits::default(), 1).unwrap();
        let request = loader.request(&missing_path).unwrap();
        let event = loader.wait_next(Duration::from_secs(2)).unwrap();

        assert_eq!(event.id, request);
        assert_eq!(event.path, missing_path);
        assert!(event.result.is_err());
        drop(loader);
        let _ = fs::remove_file(event.path);
    }
}
