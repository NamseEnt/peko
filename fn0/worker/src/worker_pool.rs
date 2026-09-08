use anyhow::Result;
use bytes::Bytes;
use fn0::cache::BundleCache;
use fn0::{CodeExecutor, ExecutionContext, RequestCancellation, panic_payload_string};
use futures::FutureExt;
use http_body_util::combinators::UnsyncBoxBody;
use std::hash::Hasher;
use std::panic::AssertUnwindSafe;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread;
use tokio::sync::{mpsc, oneshot};

pub type Body = UnsyncBoxBody<Bytes, anyhow::Error>;
pub type Request = hyper::Request<Body>;
pub type Response = hyper::Response<Body>;

pub struct RequestEnvelope {
    pub project_id: String,
    pub req: Request,
    pub resp_tx: oneshot::Sender<Result<Response>>,
    pub enqueued_at: std::time::Instant,
    pub execution_deadline: std::time::Duration,
    admission: Option<ProjectAdmissionGuard>,
    started_sender: Option<oneshot::Sender<()>>,
}

impl RequestEnvelope {
    pub fn new(
        project_id: String,
        req: Request,
        resp_tx: oneshot::Sender<Result<Response>>,
    ) -> Self {
        Self {
            project_id,
            req,
            resp_tx,
            enqueued_at: std::time::Instant::now(),
            execution_deadline: PROJECT_EXECUTION_DEADLINE,
            admission: None,
            started_sender: None,
        }
    }

    pub fn with_execution_deadline(mut self, execution_deadline: std::time::Duration) -> Self {
        self.execution_deadline = execution_deadline;
        self
    }

    pub fn with_start_signal(mut self) -> (Self, oneshot::Receiver<()>) {
        let (started_sender, started_receiver) = oneshot::channel();
        self.started_sender = Some(started_sender);
        (self, started_receiver)
    }

    #[cfg(test)]
    pub(crate) fn signal_started(&mut self) {
        if let Some(started_sender) = self.started_sender.take() {
            let _ = started_sender.send(());
        }
    }
}

#[derive(Debug)]
pub enum DispatchError {
    Full,
    Closed,
}

const QUEUE_CAPACITY: usize = 256;
const PROJECT_ACTIVE_LIMIT: usize = 32;
const PROJECT_WAITING_LIMIT: usize = 128;
const PROJECT_WAIT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(15);
const PROJECT_EXECUTION_DEADLINE: std::time::Duration = std::time::Duration::from_secs(15);
const SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

struct ProjectAdmission {
    active: Arc<tokio::sync::Semaphore>,
    outstanding: AtomicUsize,
}

struct ProjectAdmissionGuard {
    project_id: String,
    admission: Arc<ProjectAdmission>,
}

impl Drop for ProjectAdmissionGuard {
    fn drop(&mut self) {
        if self.admission.outstanding.fetch_sub(1, Ordering::AcqRel) == 1 {
            project_admissions().remove_if(&self.project_id, |_, admission| {
                Arc::ptr_eq(admission, &self.admission)
                    && admission.outstanding.load(Ordering::Acquire) == 0
            });
        }
    }
}

fn project_admissions() -> &'static dashmap::DashMap<String, Arc<ProjectAdmission>> {
    static PROJECT_ADMISSIONS: OnceLock<dashmap::DashMap<String, Arc<ProjectAdmission>>> =
        OnceLock::new();
    PROJECT_ADMISSIONS.get_or_init(dashmap::DashMap::new)
}

fn reserve_project(project_id: &str) -> Result<ProjectAdmissionGuard, DispatchError> {
    let admission = project_admissions()
        .entry(project_id.to_string())
        .or_insert_with(|| {
            Arc::new(ProjectAdmission {
                active: Arc::new(tokio::sync::Semaphore::new(PROJECT_ACTIVE_LIMIT)),
                outstanding: AtomicUsize::new(0),
            })
        })
        .clone();
    admission
        .outstanding
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |outstanding| {
            (outstanding < PROJECT_ACTIVE_LIMIT + PROJECT_WAITING_LIMIT).then_some(outstanding + 1)
        })
        .map_err(|_| DispatchError::Full)?;
    Ok(ProjectAdmissionGuard {
        project_id: project_id.to_string(),
        admission,
    })
}

pub fn spawn_workers<C>(
    ctx: Arc<ExecutionContext<C>>,
    num_threads: usize,
) -> Vec<mpsc::Sender<RequestEnvelope>>
where
    C: BundleCache,
{
    assert!(num_threads > 0, "worker pool must have at least one thread");
    let mut senders = Vec::with_capacity(num_threads);

    for idx in 0..num_threads {
        let (tx, rx) = mpsc::channel::<RequestEnvelope>(QUEUE_CAPACITY);
        senders.push(tx);
        let ctx = ctx.clone();
        thread::Builder::new()
            .name(format!("fn0-worker-{idx}"))
            .spawn(move || run_worker(idx, ctx, rx))
            .expect("failed to spawn worker thread");
    }

    senders
}

pub fn dispatch(
    senders: &[mpsc::Sender<RequestEnvelope>],
    mut env: RequestEnvelope,
) -> Result<(), DispatchError> {
    env.admission = Some(reserve_project(&env.project_id)?);
    let idx = pick_worker(&env.project_id, senders.len());
    match senders[idx].try_send(env) {
        Ok(()) => Ok(()),
        Err(mpsc::error::TrySendError::Full(_)) => Err(DispatchError::Full),
        Err(mpsc::error::TrySendError::Closed(_)) => Err(DispatchError::Closed),
    }
}

fn pick_worker(project_id: &str, n: usize) -> usize {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hasher.write(project_id.as_bytes());
    (hasher.finish() as usize) % n
}

fn run_worker<C>(idx: usize, ctx: Arc<ExecutionContext<C>>, mut rx: mpsc::Receiver<RequestEnvelope>)
where
    C: BundleCache,
{
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .thread_name(format!("fn0-worker-{idx}"))
        .build()
        .expect("failed to build current_thread runtime");
    let local = tokio::task::LocalSet::new();

    rt.block_on(local.run_until(async move {
        let executor = Rc::new(CodeExecutor::new(ctx));

        let sweep_executor = executor.clone();
        tokio::task::spawn_local(async move {
            let mut interval = tokio::time::interval(SWEEP_INTERVAL);
            interval.tick().await;
            loop {
                interval.tick().await;
                sweep_executor.sweep_unregistered().await;
            }
        });

        while let Some(env) = rx.recv().await {
            fn0::telemetry::stage_duration("queue_wait", env.enqueued_at.elapsed());
            let executor = executor.clone();
            tokio::task::spawn_local(async move {
                let RequestEnvelope {
                    project_id,
                    mut req,
                    resp_tx,
                    enqueued_at: _,
                    execution_deadline,
                    admission,
                    started_sender,
                } = env;
                let Some(admission) = admission else {
                    let _ = resp_tx.send(Err(anyhow::anyhow!("project admission missing")));
                    return;
                };
                let active = admission.admission.active.clone();
                let cancellation = req
                    .extensions()
                    .get::<RequestCancellation>()
                    .map(|value| value.0.clone())
                    .unwrap_or_else(tokio_util::sync::CancellationToken::new);
                if req.extensions().get::<RequestCancellation>().is_none() {
                    req.extensions_mut()
                        .insert(RequestCancellation(cancellation.clone()));
                }
                let active_permit = tokio::select! {
                    _ = cancellation.cancelled() => {
                        drop(admission);
                        let _ = resp_tx.send(Err(anyhow::anyhow!("request cancelled")));
                        return;
                    }
                    active_result = tokio::time::timeout(PROJECT_WAIT_DEADLINE, active.acquire_owned()) => {
                        match active_result {
                            Ok(Ok(active_permit)) => active_permit,
                            Ok(Err(_)) => {
                                let _ = resp_tx.send(Err(anyhow::anyhow!("project admission closed")));
                                return;
                            }
                            Err(_) => {
                                let _ = resp_tx.send(Err(anyhow::anyhow!("project admission timeout")));
                                return;
                            }
                        }
                    }
                };
                if let Some(started_sender) = started_sender {
                    let _ = started_sender.send(());
                }
                let outcome = tokio::select! {
                    _ = cancellation.cancelled() => {
                        drop(active_permit);
                        drop(admission);
                        let _ = resp_tx.send(Err(anyhow::anyhow!("request cancelled")));
                        return;
                    }
                    outcome = tokio::time::timeout(
                        execution_deadline,
                        AssertUnwindSafe(executor.run(&project_id, "/", req, None)).catch_unwind(),
                    ) => outcome,
                };
                drop(active_permit);
                drop(admission);
                match outcome {
                    Ok(Ok(result)) => {
                        if resp_tx.send(result).is_err() {
                            cancellation.cancel();
                            fn0::telemetry::oneshot_drop_before_response();
                        }
                    }
                    Ok(Err(panic)) => {
                        cancellation.cancel();
                        let panic_msg = panic_payload_string(&panic);
                        fn0::telemetry::panicked();
                        tracing::error!(
                            %project_id,
                            panic = %panic_msg,
                            "executor panicked; response channel dropped"
                        );
                    }
                    Err(_) => {
                        cancellation.cancel();
                        fn0::telemetry::request_deadline_exceeded();
                        let _ = resp_tx
                            .send(Err(anyhow::anyhow!("request execution deadline exceeded")));
                    }
                }
            });
        }
    }));

    tracing::info!(worker = idx, "worker thread exiting");
}

pub fn default_num_threads() -> usize {
    if let Ok(s) = std::env::var("FN0_WORKER_THREADS")
        && let Ok(n) = s.parse::<usize>()
        && n > 0
    {
        return n;
    }
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_admission_bounds_active_and_waiting_work() {
        let project_id = "project-admission-bounds-test";
        let reservations: Vec<ProjectAdmissionGuard> = (0..PROJECT_ACTIVE_LIMIT
            + PROJECT_WAITING_LIMIT)
            .map(|_| reserve_project(project_id).expect("reservation"))
            .collect();
        assert!(matches!(
            reserve_project(project_id),
            Err(DispatchError::Full)
        ));
        drop(reservations);
        assert!(reserve_project(project_id).is_ok());
    }
}
