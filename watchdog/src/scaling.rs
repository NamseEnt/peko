use crate::{health_recorder::*, worker_infra::*, *};

pub async fn try_scale_out(
    context: &Context,
    health_records: HealthRecords,
    worker_health_response_map: WorkerHealthResponseMap,
    worker_infra: &dyn WorkerInfra,
) -> color_eyre::Result<()> {
    let starting_workers = health_records.iter().filter_map(|(worker_id, record)| {
        if let HealthState::Starting = record.state {
            Some(worker_id.clone())
        } else {
            None
        }
    });

    let (old_starting_workers, fresh_starting_workers): (Vec<_>, Vec<_>) = starting_workers
        .partition(|worker_id| {
            let (info, _response) = worker_health_response_map.get(worker_id).unwrap();
            context.start_time - info.instance_created > context.max_start_timeout
        });

    let alive_worker_len = health_records
        .iter()
        .filter(|(_, record)| match record.state {
            HealthState::Starting
            | HealthState::Healthy { .. }
            | HealthState::RetryingCheck { .. }
            | HealthState::MarkedForTermination
            | HealthState::GracefulShuttingDown => true,
            HealthState::TerminatedConfirm | HealthState::InvisibleOnInfra => false,
        })
        .count();

    println!("alive_worker_len: {alive_worker_len}");
    println!("old_starting_workers: {old_starting_workers:?}");
    println!("fresh_starting_workers: {fresh_starting_workers:?}");

    let terminate_olds = futures::stream::iter(old_starting_workers).for_each_concurrent(
        16,
        |worker_id| async move {
            let _ = worker_infra.terminate(&worker_id).await;
        },
    );

    let start_new = async move {
        let Some(left_starting_count) = context
            .max_starting_count
            .checked_sub(fresh_starting_workers.len())
        else {
            return color_eyre::eyre::Ok(());
        };

        println!("left_starting_count: {left_starting_count}");

        if left_starting_count == 0 {
            println!("left_starting_count == 0. No more starting workers allowed");
            return Ok(());
        }

        if alive_worker_len >= 1 {
            println!("alive_worker_len >= 1. No space to launch new worker instance");
            return Ok(());
        }

        println!("Launching new worker instance");
        worker_infra.launch_instances(1).await?;
        Ok(())
    };

    futures::try_join!(terminate_olds.map(|_| Ok(())), start_new)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
    };

    #[derive(Default)]
    struct MockWorkerInfra {
        terminated_workers: Arc<Mutex<Vec<WorkerId>>>,
        launched_instances: Arc<Mutex<usize>>,
    }

    impl WorkerInfra for MockWorkerInfra {
        fn get_worker_infos<'a>(
            &'a self,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = color_eyre::Result<WorkerInfos>> + 'a + Send>,
        > {
            Box::pin(async { Ok(vec![]) })
        }

        fn terminate<'a>(
            &'a self,
            worker_id: &'a WorkerId,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = color_eyre::Result<()>> + 'a + Send>>
        {
            let terminated = self.terminated_workers.clone();
            let id = worker_id.clone();
            Box::pin(async move {
                terminated.lock().unwrap().push(id);
                Ok(())
            })
        }

        fn launch_instances<'a>(
            &'a self,
            count: usize,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = color_eyre::Result<()>> + 'a + Send>>
        {
            let launched = self.launched_instances.clone();
            Box::pin(async move {
                *launched.lock().unwrap() += count;
                Ok(())
            })
        }
    }

    fn create_context(max_start_timeout: chrono::Duration, max_starting_count: usize) -> Context {
        Context {
            start_time: Utc::now(),
            domain: "example.com".to_string(),
            max_graceful_shutdown_wait_time: chrono::Duration::seconds(10),
            max_healthy_check_retrials: 3,
            max_start_timeout,
            max_starting_count,
        }
    }

    fn create_worker_info(id: &str, created_ago: chrono::Duration) -> WorkerInfo {
        WorkerInfo {
            id: WorkerId(id.to_string()),
            instance_created: Utc::now() - created_ago,
            ip: None,
            instance_state: WorkerInstanceState::Starting,
        }
    }

    fn create_health_record(state: HealthState) -> HealthRecord {
        HealthRecord {
            state,
            state_transited_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn test_terminate_old_starting_workers() {
        let max_timeout = chrono::Duration::seconds(60);
        let context = create_context(max_timeout, 5);

        let mut health_records = BTreeMap::new();
        let mut response_map = BTreeMap::new();

        let old_id = WorkerId("old_worker".to_string());
        let fresh_id = WorkerId("fresh_worker".to_string());

        // Old worker: created 70s ago (timeout 60s)
        health_records.insert(old_id.clone(), create_health_record(HealthState::Starting));
        response_map.insert(
            old_id.clone(),
            (
                create_worker_info("old_worker", chrono::Duration::seconds(70)),
                None,
            ),
        );

        // Fresh worker: created 30s ago
        health_records.insert(
            fresh_id.clone(),
            create_health_record(HealthState::Starting),
        );
        response_map.insert(
            fresh_id.clone(),
            (
                create_worker_info("fresh_worker", chrono::Duration::seconds(30)),
                None,
            ),
        );

        let infra = MockWorkerInfra::default();

        try_scale_out(&context, health_records, response_map, &infra)
            .await
            .unwrap();

        let terminated = infra.terminated_workers.lock().unwrap();
        assert_eq!(terminated.len(), 1);
        assert_eq!(terminated[0], old_id);

        let launched = *infra.launched_instances.lock().unwrap();
        // 2 alive (old + fresh), limit 1. No launch.
        assert_eq!(launched, 0);
    }

    #[tokio::test]
    async fn test_count_alive_workers_and_launch() {
        let context = create_context(chrono::Duration::seconds(60), 5);

        // No workers
        let health_records = BTreeMap::new();
        let response_map = BTreeMap::new();
        let infra = MockWorkerInfra::default();

        try_scale_out(&context, health_records, response_map, &infra)
            .await
            .unwrap();

        // Alive = 0 < 1. Should launch.
        assert_eq!(*infra.launched_instances.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn test_do_not_launch_if_alive_limit_reached() {
        let context = create_context(chrono::Duration::seconds(60), 5);

        let mut health_records = BTreeMap::new();
        let healthy_id = WorkerId("healthy".to_string());

        health_records.insert(
            healthy_id,
            HealthRecord {
                state: HealthState::Healthy {
                    ip: "127.0.0.1".parse().unwrap(),
                },
                state_transited_at: Utc::now(),
            },
        );

        let response_map = BTreeMap::new();
        let infra = MockWorkerInfra::default();

        try_scale_out(&context, health_records, response_map, &infra)
            .await
            .unwrap();

        assert_eq!(*infra.launched_instances.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn test_do_not_launch_if_max_starting_reached() {
        let context = create_context(chrono::Duration::seconds(60), 1); // Limit 1

        let mut health_records = BTreeMap::new();
        let mut response_map = BTreeMap::new();
        let id = WorkerId("starting".to_string());

        health_records.insert(id.clone(), create_health_record(HealthState::Starting));
        response_map.insert(
            id.clone(),
            (
                create_worker_info("starting", chrono::Duration::seconds(10)),
                None,
            ),
        );

        let infra = MockWorkerInfra::default();

        try_scale_out(&context, health_records, response_map, &infra)
            .await
            .unwrap();

        // Fresh starting = 1. Max starting = 1.
        // left = 0. No launch.
        assert_eq!(*infra.launched_instances.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn test_terminate_and_launch() {
        // Scenario: 1 old starting worker (timeout), 0 fresh. Alive count logic?
        // Old starting worker is "Starting" so it counts as alive in `alive_worker_len` initially?
        // Code:
        // `alive_worker_len` filters `health_records`.
        // `HealthState::Starting` (old) -> true.
        // So alive = 1.
        // If alive >= 1, no launch.
        // So even if we terminate old, we don't launch new in the SAME tick?
        // `terminate_olds` runs concurrently with `start_new`?
        // `futures::try_join!`. They run concurrently.
        // `start_new` calculates `alive_worker_len` BEFORE `terminate_olds` finishes (it's pre-calculated).
        // `alive_worker_len` is variable.
        // Line 23: `let alive_worker_len = ...`. It is calculated synchronously before the join.
        // So `alive_worker_len` INCLUDES the old worker that is about to be terminated.
        // So if we have 1 old worker, alive=1. Launch logic sees alive=1 >= 1 -> No launch.
        // This means it takes 2 ticks to replace a stuck worker?
        // Tick 1: Terminate stuck worker. (It is still in health records as Starting).
        // Tick 2: Stuck worker is gone (or state changed)?
        // If state is still Starting, it repeats?
        // `terminate` call sends signal.
        // `health_records` are passed in.
        // Next tick, `health_records` should be updated?
        // If `WorkerInfra::terminate` is called, the worker might be removed or state changed to Terminating?
        // `update_health_records` handles state transitions.
        // But `try_scale_out` just sends terminate.

        // Let's verify this behavior in test.

        let context = create_context(chrono::Duration::seconds(60), 5);
        let mut health_records = BTreeMap::new();
        let mut response_map = BTreeMap::new();
        let old_id = WorkerId("old".to_string());

        health_records.insert(old_id.clone(), create_health_record(HealthState::Starting));
        response_map.insert(
            old_id.clone(),
            (
                create_worker_info("old", chrono::Duration::seconds(70)),
                None,
            ),
        );

        let infra = MockWorkerInfra::default();
        try_scale_out(&context, health_records, response_map, &infra)
            .await
            .unwrap();

        assert_eq!(infra.terminated_workers.lock().unwrap().len(), 1);
        assert_eq!(*infra.launched_instances.lock().unwrap(), 0);
    }
}
