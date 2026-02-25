//! In-memory scheduler: single queue + one worker, job state in a map.

use crate::{Scheduler, SchedulerError};
use async_trait::async_trait;
use chrono::Utc;
use mem_types::{ApiAddRequest, AuditEvent, AuditEventKind, AuditStore, Job, JobStatus};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use uuid::Uuid;

struct JobState {
    job: Job,
}

/// In-memory scheduler: queues add requests, one worker calls MemCube::add_memories and updates job status.
pub struct InMemoryScheduler {
    jobs: Arc<RwLock<HashMap<String, JobState>>>,
    tx: mpsc::UnboundedSender<(String, ApiAddRequest)>,
}

impl InMemoryScheduler {
    /// Create scheduler and spawn worker. Worker runs on the given MemCube.
    /// If `audit_store` is provided, an AuditEvent(Add) is appended when an add job completes successfully.
    pub fn new(
        cube: Arc<dyn mem_types::MemCube + Send + Sync>,
        audit_store: Option<Arc<dyn AuditStore + Send + Sync>>,
    ) -> Self {
        let jobs: Arc<RwLock<HashMap<String, JobState>>> = Arc::new(RwLock::new(HashMap::new()));
        let (tx, mut rx) = mpsc::unbounded_channel::<(String, ApiAddRequest)>();

        let jobs_clone = Arc::clone(&jobs);
        let audit_store = audit_store.clone();
        tokio::spawn(async move {
            while let Some((job_id, req)) = rx.recv().await {
                let now = Utc::now().to_rfc3339();
                {
                    let mut guard = jobs_clone.write().await;
                    if let Some(s) = guard.get_mut(&job_id) {
                        s.job.status = JobStatus::Running;
                        s.job.updated_at = now.clone();
                    }
                }
                let result = cube.add_memories(&req).await;
                let now2 = Utc::now().to_rfc3339();
                let (status, result_summary) = match &result {
                    Ok(res) => (
                        JobStatus::Done,
                        Some(serde_json::json!({ "code": res.code, "message": res.message })),
                    ),
                    Err(e) => (
                        JobStatus::Failed,
                        Some(serde_json::json!({ "error": e.to_string() })),
                    ),
                };
                if let Ok(ref res) = result {
                    if let Some(ref store) = audit_store {
                        let cube_ids = req.writable_cube_ids();
                        let memory_id = res
                            .data
                            .as_ref()
                            .and_then(|d| d.first())
                            .and_then(|v| v.get("id"))
                            .and_then(|v| v.as_str())
                            .map(String::from);
                        let event = AuditEvent {
                            event_id: Uuid::new_v4().to_string(),
                            kind: AuditEventKind::Add,
                            memory_id,
                            user_id: req.user_id.clone(),
                            cube_id: cube_ids
                                .first()
                                .cloned()
                                .unwrap_or_else(|| req.user_id.clone()),
                            timestamp: now2.clone(),
                            input_summary: None,
                            outcome: Some(format!("code={}", res.code)),
                        };
                        let _ = store.append(event).await;
                    }
                }
                let mut guard = jobs_clone.write().await;
                if let Some(s) = guard.get_mut(&job_id) {
                    s.job.status = status;
                    s.job.updated_at = now2;
                    s.job.result_summary = result_summary;
                }
            }
        });

        Self { jobs, tx }
    }

    fn now_iso(&self) -> String {
        Utc::now().to_rfc3339()
    }
}

#[async_trait]
impl Scheduler for InMemoryScheduler {
    async fn submit_add(&self, req: ApiAddRequest) -> Result<String, SchedulerError> {
        let job_id = Uuid::new_v4().to_string();
        let now = self.now_iso();
        let job = Job {
            job_id: job_id.clone(),
            status: JobStatus::Pending,
            created_at: now.clone(),
            updated_at: now,
            result_summary: None,
        };
        {
            let mut guard = self.jobs.write().await;
            guard.insert(job_id.clone(), JobState { job });
        }
        self.tx
            .send((job_id.clone(), req))
            .map_err(|_| SchedulerError::Other("worker channel closed".to_string()))?;
        Ok(job_id)
    }

    async fn get_status(&self, job_id: &str) -> Result<Option<Job>, SchedulerError> {
        let guard = self.jobs.read().await;
        Ok(guard.get(job_id).map(|s| s.job.clone()))
    }
}
