//! Scheduler trait: submit add job, get status.

use async_trait::async_trait;
use mem_types::{ApiAddRequest, Job};

#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    #[error("scheduler error: {0}")]
    Other(String),
    #[error("job not found: {0}")]
    JobNotFound(String),
}

/// Scheduler for async add: submit returns job_id, status can be polled.
///
/// Contract: `get_status` returns `Ok(None)` when the job_id is unknown (e.g. not yet created or
/// evicted). The API layer should map `Ok(None)` to HTTP 404 for consistent semantics.
#[async_trait]
pub trait Scheduler: Send + Sync {
    /// Submit an add request; returns job_id. When async, the actual add runs in a worker.
    async fn submit_add(&self, req: ApiAddRequest) -> Result<String, SchedulerError>;

    /// Get current job status by user_id + job_id (task_id).
    /// Returns `Ok(None)` when job is unknown or not owned by the given user.
    async fn get_status(&self, user_id: &str, job_id: &str) -> Result<Option<Job>, SchedulerError>;
}
