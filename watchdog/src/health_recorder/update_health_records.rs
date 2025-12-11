use super::*;
use crate::{worker_infra::WorkerHealthKind, *};

pub fn update_health_records(
    context: &Context,
    health_records: &mut HealthRecords,
    worker_health_response_map: WorkerHealthResponseMap,
) -> color_eyre::Result<()> {
    let &Context {
        start_time,
        max_graceful_shutdown_wait_time,
        max_healthy_check_retrials,
        max_start_timeout,
        ..
    } = context;

    let worker_health_response_not_in_records = worker_health_response_map
        .iter()
        .filter(|(id, _)| !health_records.contains_key(id));

    let mut new_worker_records = vec![];

    for (worker_id, (worker_info, worker_status)) in worker_health_response_not_in_records {
        new_worker_records.push((
            worker_id.clone(),
            HealthRecord {
                state: {
                    match worker_info.instance_state {
                        WorkerInstanceState::Starting => HealthState::Starting,
                        WorkerInstanceState::Running => match worker_status {
                            Some(worker_health_response) => match worker_health_response.kind {
                                WorkerHealthKind::Good => HealthState::Healthy {
                                    ip: worker_health_response.ip,
                                },
                                WorkerHealthKind::GracefulShuttingDown => {
                                    HealthState::GracefulShuttingDown
                                }
                            },
                            None => HealthState::RetryingCheck { retrials: 1 },
                        },
                        WorkerInstanceState::Terminating => continue,
                    }
                },
                state_transited_at: start_time,
            },
        ));
    }

    health_records.retain(|worker_id, record| {
        let Some((worker_info, health_response)) = worker_health_response_map.get(worker_id) else {
            match record.state {
                HealthState::Starting
                | HealthState::Healthy { .. }
                | HealthState::RetryingCheck { .. }
                | HealthState::GracefulShuttingDown => {
                    record.state = HealthState::InvisibleOnInfra;
                    record.state_transited_at = start_time;
                    return true;
                }
                HealthState::MarkedForTermination
                | HealthState::TerminatedConfirm
                | HealthState::InvisibleOnInfra => {
                    return start_time - record.state_transited_at < TimeDelta::minutes(5);
                }
            }
        };

        if let WorkerInstanceState::Terminating = worker_info.instance_state
            && !matches!(record.state, HealthState::TerminatedConfirm)
        {
            record.state = HealthState::TerminatedConfirm;
            record.state_transited_at = start_time;
            return true;
        }

        match health_response {
            Some(worker_health_response) => match worker_health_response.kind {
                WorkerHealthKind::Good => match record.state {
                    HealthState::GracefulShuttingDown => {
                        // Once in GracefulShuttingDown, we should not revert to Healthy even if the worker reports Good.
                        // We might optionally update the timestamp to reflect it's still alive, but keeping the state is key.
                        // For now, let's just do nothing to preserve the shutdown state.
                    }
                    _ => {
                        record.state = HealthState::Healthy {
                            ip: worker_health_response.ip,
                        };
                        record.state_transited_at = start_time;
                    }
                },
                WorkerHealthKind::GracefulShuttingDown => match record.state {
                    HealthState::Starting
                    | HealthState::Healthy { .. }
                    | HealthState::RetryingCheck { .. }
                    | HealthState::MarkedForTermination
                    | HealthState::InvisibleOnInfra => {
                        record.state = HealthState::GracefulShuttingDown;
                        record.state_transited_at = start_time;
                    }
                    HealthState::TerminatedConfirm => {
                        eprintln!(
                            "Unexpected state: Terminated confirmed but graceful shutting down"
                        );
                        record.state = HealthState::GracefulShuttingDown;
                        record.state_transited_at = start_time;
                    }
                    HealthState::GracefulShuttingDown => {}
                },
            },
            None => match &mut record.state {
                HealthState::Starting => {
                    if record.state_transited_at + max_start_timeout < start_time {
                        record.state = HealthState::MarkedForTermination;
                        record.state_transited_at = start_time;
                    }
                }
                HealthState::Healthy { .. } | HealthState::InvisibleOnInfra => {
                    record.state = HealthState::RetryingCheck { retrials: 1 };
                    record.state_transited_at = start_time;
                }
                HealthState::RetryingCheck { retrials } => {
                    *retrials += 1;
                    if *retrials > max_healthy_check_retrials {
                        record.state = HealthState::MarkedForTermination;
                        record.state_transited_at = start_time;
                    }
                }
                HealthState::MarkedForTermination
                | HealthState::GracefulShuttingDown
                | HealthState::TerminatedConfirm => {}
            },
        }

        if let HealthState::GracefulShuttingDown = record.state
            && record.state_transited_at + max_graceful_shutdown_wait_time < start_time
        {
            record.state = HealthState::MarkedForTermination;
            record.state_transited_at = start_time;
        }

        true
    });

    health_records.extend(new_worker_records);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker_infra::{
        WorkerHealthKind, WorkerHealthResponse, WorkerInfo, WorkerInstanceState,
    };
    use chrono::{TimeDelta, Utc};

    // ========================================
    // Test Fixture
    // ========================================

    struct TestFixture {
        context: Context,
        health_records: HealthRecords,
        response_map: WorkerHealthResponseMap,
        start_time: DateTime<Utc>,
    }

    impl TestFixture {
        fn new() -> Self {
            let start_time = Utc::now();
            let context = Context {
                start_time,
                domain: "test.example.com".to_string(),
                max_graceful_shutdown_wait_time: TimeDelta::minutes(5),
                max_healthy_check_retrials: 3,
                max_start_timeout: TimeDelta::minutes(10),
                max_starting_count: 5,
            };
            Self {
                context,
                health_records: HealthRecords::new(),
                response_map: WorkerHealthResponseMap::new(),
                start_time,
            }
        }

        fn start_time(&self) -> DateTime<Utc> {
            self.start_time
        }

        fn previous_time(&self, minutes_ago: i64) -> DateTime<Utc> {
            self.start_time - TimeDelta::minutes(minutes_ago)
        }

        // Builder methods
        fn with_record(mut self, id: &str, state: HealthState, time_offset: TimeDelta) -> Self {
            self.health_records.insert(
                WorkerId(id.to_string()),
                HealthRecord {
                    state,
                    state_transited_at: self.start_time + time_offset,
                },
            );
            self
        }

        fn with_response(
            mut self,
            id: &str,
            state: WorkerInstanceState,
            health_response: Option<WorkerHealthKind>,
        ) -> Self {
            let worker_info = WorkerInfo {
                id: WorkerId(id.to_string()),
                ip: None,
                instance_state: state,
                instance_created: Utc::now(), // irrelevant for these tests
            };

            let response = health_response.map(|kind| WorkerHealthResponse {
                kind,
                ip: "127.0.0.1".parse().unwrap(),
            });

            self.response_map
                .insert(WorkerId(id.to_string()), (worker_info, response));
            self
        }

        fn run(mut self) -> Self {
            update_health_records(
                &self.context,
                &mut self.health_records,
                self.response_map.clone(),
            )
            .unwrap();
            self
        }

        // Assertions
        fn get_record(&self, id: &str) -> &HealthRecord {
            self.health_records
                .get(&WorkerId(id.to_string()))
                .expect("Record expected but not found")
        }

        fn assert_state_matches<P>(self, id: &str, predicate: P) -> Self
        where
            P: FnOnce(&HealthState) -> bool,
        {
            let record = self.get_record(id);
            assert!(
                predicate(&record.state),
                "State predicate failed for {}",
                id
            );
            self
        }

        fn assert_transited_at(self, id: &str, expected_time: DateTime<Utc>) -> Self {
            let record = self.get_record(id);
            assert_eq!(
                record.state_transited_at, expected_time,
                "TransitedAt time mismatch for {}",
                id
            );
            self
        }

        fn assert_no_record(self, id: &str) -> Self {
            assert!(
                !self.health_records.contains_key(&WorkerId(id.to_string())),
                "Record should NOT exist for {}",
                id
            );
            self
        }
    }

    // ========================================
    // Helper Macro for State Transitions
    // ========================================

    macro_rules! test_transition {
        ($name:ident, $start_state:expr, $worker_state:expr, $health_resp:expr, $matcher:pat) => {
            #[test]
            fn $name() {
                let fixture = TestFixture::new();
                let start_time = fixture.start_time();

                fixture
                    .with_record("worker1", $start_state, TimeDelta::minutes(-1))
                    .with_response("worker1", $worker_state, $health_resp)
                    .run()
                    .assert_state_matches("worker1", |s| matches!(s, $matcher))
                    .assert_transited_at("worker1", start_time);
            }
        };
    }

    // For cases where time should be preserved
    macro_rules! test_state_preserved {
        ($name:ident, $start_state:expr, $worker_state:expr, $health_resp:expr, $matcher:pat) => {
            #[test]
            fn $name() {
                let fixture = TestFixture::new();
                let previous_time = fixture.previous_time(1);

                fixture
                    .with_record("worker1", $start_state, TimeDelta::minutes(-1))
                    .with_response("worker1", $worker_state, $health_resp)
                    .run()
                    .assert_state_matches("worker1", |s| matches!(s, $matcher))
                    .assert_transited_at("worker1", previous_time);
            }
        };
    }

    // Helper for ip
    fn test_ip() -> std::net::IpAddr {
        "127.0.0.1".parse().unwrap()
    }

    // ========================================
    // 1. New Workers Discovery
    // ========================================

    #[test]
    fn test_new_worker_with_good_response() {
        let fixture = TestFixture::new();
        let start_time = fixture.start_time();

        fixture
            .with_response(
                "worker1",
                WorkerInstanceState::Running,
                Some(WorkerHealthKind::Good),
            )
            .run()
            .assert_state_matches("worker1", |s| matches!(s, HealthState::Healthy { .. }))
            .assert_transited_at("worker1", start_time);
    }

    #[test]
    fn test_new_worker_with_graceful_shutdown() {
        let fixture = TestFixture::new();
        let start_time = fixture.start_time();

        fixture
            .with_response(
                "worker1",
                WorkerInstanceState::Running,
                Some(WorkerHealthKind::GracefulShuttingDown),
            )
            .run()
            .assert_state_matches("worker1", |s| {
                matches!(s, HealthState::GracefulShuttingDown)
            })
            .assert_transited_at("worker1", start_time);
    }

    #[test]
    fn test_new_worker_with_no_response() {
        let fixture = TestFixture::new();
        let start_time = fixture.start_time();

        fixture
            .with_response("worker1", WorkerInstanceState::Running, None)
            .run()
            .assert_state_matches("worker1", |s| {
                matches!(s, HealthState::RetryingCheck { retrials: 1 })
            })
            .assert_transited_at("worker1", start_time);
    }

    // ========================================
    // 2. Existing Worker State Updates (Happy Path)
    // ========================================

    test_transition!(
        test_healthy_worker_stays_healthy,
        HealthState::Healthy { ip: test_ip() },
        WorkerInstanceState::Running,
        Some(WorkerHealthKind::Good),
        HealthState::Healthy { .. }
    );

    test_transition!(
        test_retrying_worker_recovers,
        HealthState::RetryingCheck { retrials: 2 },
        WorkerInstanceState::Running,
        Some(WorkerHealthKind::Good),
        HealthState::Healthy { .. }
    );

    test_transition!(
        test_healthy_worker_receives_graceful_shutdown,
        HealthState::Healthy { ip: test_ip() },
        WorkerInstanceState::Running,
        Some(WorkerHealthKind::GracefulShuttingDown),
        HealthState::GracefulShuttingDown
    );

    // ========================================
    // 3. Health Check Failures and Retry Logic
    // ========================================

    test_transition!(
        test_healthy_worker_first_failure,
        HealthState::Healthy { ip: test_ip() },
        WorkerInstanceState::Running,
        None,
        HealthState::RetryingCheck { retrials: 1 }
    );

    test_state_preserved!(
        test_retrying_worker_increases_retrials,
        HealthState::RetryingCheck { retrials: 2 },
        WorkerInstanceState::Running,
        None,
        HealthState::RetryingCheck { retrials: 3 }
    );

    test_transition!(
        test_max_retrials_exceeded,
        HealthState::RetryingCheck { retrials: 3 },
        WorkerInstanceState::Running,
        None,
        HealthState::MarkedForTermination
    );

    test_state_preserved!(
        test_marked_for_termination_stays_unchanged_on_no_response,
        HealthState::MarkedForTermination,
        WorkerInstanceState::Running,
        None,
        HealthState::MarkedForTermination
    );

    test_state_preserved!(
        test_graceful_shutting_down_stays_unchanged_on_no_response,
        HealthState::GracefulShuttingDown,
        WorkerInstanceState::Running,
        None,
        HealthState::GracefulShuttingDown
    );

    test_state_preserved!(
        test_terminated_confirm_stays_unchanged_on_no_response,
        HealthState::TerminatedConfirm,
        WorkerInstanceState::Running,
        None,
        HealthState::TerminatedConfirm
    );

    // ========================================
    // 4. Infrastructure Sync (Disappeared Workers)
    // ========================================

    #[test]
    fn test_workers_disappear_from_infra() {
        // Covering Case 4.1 for various states
        let fixture = TestFixture::new();
        let start_time = fixture.start_time();

        fixture
            .with_record(
                "healthy",
                HealthState::Healthy { ip: test_ip() },
                TimeDelta::minutes(-1),
            )
            .with_record(
                "retrying",
                HealthState::RetryingCheck { retrials: 2 },
                TimeDelta::minutes(-1),
            )
            .with_record(
                "graceful",
                HealthState::GracefulShuttingDown,
                TimeDelta::minutes(-1),
            )
            .run()
            .assert_state_matches("healthy", |s| matches!(s, HealthState::InvisibleOnInfra))
            .assert_transited_at("healthy", start_time)
            .assert_state_matches("retrying", |s| matches!(s, HealthState::InvisibleOnInfra))
            .assert_transited_at("retrying", start_time)
            .assert_state_matches("graceful", |s| matches!(s, HealthState::InvisibleOnInfra))
            .assert_transited_at("graceful", start_time);
    }

    #[test]
    fn test_invisible_worker_returns() {
        let fixture = TestFixture::new();
        let start_time = fixture.start_time();

        fixture
            .with_record(
                "worker1",
                HealthState::InvisibleOnInfra,
                TimeDelta::minutes(-1),
            )
            .with_response(
                "worker1",
                WorkerInstanceState::Running,
                Some(WorkerHealthKind::Good),
            )
            .run()
            .assert_state_matches("worker1", |s| matches!(s, HealthState::Healthy { .. }))
            .assert_transited_at("worker1", start_time);
    }

    #[test]
    fn test_invisible_worker_stays_retained_and_time_preserved() {
        // Case 4.3: Invisible state maintained & time preserved
        let fixture = TestFixture::new();
        let previous_time = fixture.previous_time(2); // 2 minutes ago

        fixture
            .with_record(
                "worker1",
                HealthState::InvisibleOnInfra,
                TimeDelta::minutes(-2),
            )
            // No response in map => invisible logic kicks in
            .run()
            .assert_state_matches("worker1", |s| matches!(s, HealthState::InvisibleOnInfra))
            .assert_transited_at("worker1", previous_time); // CRITICAL: Time must NOT be updated
    }

    // ========================================
    // 5. Cleanup / Retention Policy
    // ========================================

    #[test]
    fn test_cleanup_policy() {
        let fixture = TestFixture::new();

        fixture
            // Should be deleted (6 mins ago)
            .with_record(
                "old_term",
                HealthState::MarkedForTermination,
                TimeDelta::minutes(-6),
            )
            .with_record(
                "old_confirm",
                HealthState::TerminatedConfirm,
                TimeDelta::minutes(-6),
            )
            .with_record(
                "old_invisible",
                HealthState::InvisibleOnInfra,
                TimeDelta::minutes(-6),
            )
            // Should be retained (3 mins ago)
            .with_record(
                "recent_term",
                HealthState::MarkedForTermination,
                TimeDelta::minutes(-3),
            )
            .with_record(
                "recent_confirm",
                HealthState::TerminatedConfirm,
                TimeDelta::minutes(-3),
            )
            .with_record(
                "recent_invisible",
                HealthState::InvisibleOnInfra,
                TimeDelta::minutes(-3),
            )
            .run()
            // Check deletions
            .assert_no_record("old_term")
            .assert_no_record("old_confirm")
            .assert_no_record("old_invisible")
            // Check retentions
            .assert_state_matches("recent_term", |s| {
                matches!(s, HealthState::MarkedForTermination)
            })
            .assert_state_matches("recent_confirm", |s| {
                matches!(s, HealthState::TerminatedConfirm)
            })
            .assert_state_matches("recent_invisible", |s| {
                matches!(s, HealthState::InvisibleOnInfra)
            });
    }

    #[test]
    fn test_healthy_worker_not_deleted_when_missing_from_infra() {
        let fixture = TestFixture::new();
        let start_time = fixture.start_time();

        fixture
            .with_record(
                "worker1",
                HealthState::Healthy { ip: test_ip() },
                TimeDelta::minutes(-1),
            )
            .run() // No response
            .assert_state_matches("worker1", |s| matches!(s, HealthState::InvisibleOnInfra))
            .assert_transited_at("worker1", start_time);
    }

    // ========================================
    // 6. Graceful Shutdown Timeout
    // ========================================

    // Actually, `test_transition!` sets time to -1 minute.
    // Logic: `record.state_transited_at + max < start_time`
    // If state_transited_at = start - 6 mins.
    // -6 + 5 = -1 < 0. True.
    // If state_transited_at = start - 1 mins.
    // -1 + 5 = 4 > 0. False.
    // So `test_transition!` will NOT trigger timeout.
    // I should NOT use `test_transition!` for timeout case.
    // I will remove `test_graceful_shutdown_timeout_exceeded` macro call and use manual test.

    #[test]
    fn test_graceful_shutdown_timeout_logic() {
        let fixture = TestFixture::new();
        let start_time = fixture.start_time();
        let _previous_time_old = fixture.previous_time(6); // 6 mins ago (expired)
        let previous_time_recent = fixture.previous_time(3); // 3 mins ago (valid)

        fixture
            .with_record(
                "timeout_worker",
                HealthState::GracefulShuttingDown,
                TimeDelta::minutes(-6),
            )
            .with_response(
                "timeout_worker",
                WorkerInstanceState::Running,
                Some(WorkerHealthKind::GracefulShuttingDown),
            )
            .with_record(
                "ok_worker",
                HealthState::GracefulShuttingDown,
                TimeDelta::minutes(-3),
            )
            .with_response(
                "ok_worker",
                WorkerInstanceState::Running,
                Some(WorkerHealthKind::GracefulShuttingDown),
            )
            .run()
            .assert_state_matches("timeout_worker", |s| {
                matches!(s, HealthState::MarkedForTermination)
            })
            .assert_transited_at("timeout_worker", start_time)
            .assert_state_matches("ok_worker", |s| {
                matches!(s, HealthState::GracefulShuttingDown)
            })
            .assert_transited_at("ok_worker", previous_time_recent); // Time preserved
    }

    // ========================================
    // 7. Starting State Handling
    // ========================================

    #[test]
    fn test_new_worker_in_starting_state() {
        let fixture = TestFixture::new();
        let start_time = fixture.start_time();

        fixture
            .with_response("worker1", WorkerInstanceState::Starting, None)
            .run()
            .assert_state_matches("worker1", |s| matches!(s, HealthState::Starting))
            .assert_transited_at("worker1", start_time);
    }

    test_transition!(
        test_starting_to_healthy_transition,
        HealthState::Starting,
        WorkerInstanceState::Running,
        Some(WorkerHealthKind::Good),
        HealthState::Healthy { .. }
    );

    test_transition!(
        test_starting_to_graceful_shutdown_transition,
        HealthState::Starting,
        WorkerInstanceState::Running,
        Some(WorkerHealthKind::GracefulShuttingDown),
        HealthState::GracefulShuttingDown
    );

    test_state_preserved!(
        test_starting_state_maintained_within_timeout,
        HealthState::Starting,
        WorkerInstanceState::Starting,
        None,
        HealthState::Starting
    );

    #[test]
    fn test_starting_timeout_exceeded() {
        let fixture = TestFixture::new();
        let start_time = fixture.start_time();

        fixture
            .with_record("worker1", HealthState::Starting, TimeDelta::minutes(-11)) // 11 mins ago
            .with_response("worker1", WorkerInstanceState::Starting, None)
            .run()
            .assert_state_matches("worker1", |s| {
                matches!(s, HealthState::MarkedForTermination)
            })
            .assert_transited_at("worker1", start_time);
    }

    #[test]
    fn test_starting_worker_disappears_from_infra() {
        let fixture = TestFixture::new();
        let start_time = fixture.start_time();
        fixture
            .with_record("worker1", HealthState::Starting, TimeDelta::minutes(-1))
            .run()
            .assert_state_matches("worker1", |s| matches!(s, HealthState::InvisibleOnInfra))
            .assert_transited_at("worker1", start_time);
    }

    // ========================================
    // 8. Terminating State Handling
    // ========================================

    #[test]
    fn test_new_worker_in_terminating_state_is_ignored() {
        let fixture = TestFixture::new();
        fixture
            .with_response("worker1", WorkerInstanceState::Terminating, None)
            .run()
            .assert_no_record("worker1");
    }

    macro_rules! test_terminating_transition {
        ($name:ident, $start_state:expr) => {
            test_transition!(
                $name,
                $start_state,
                WorkerInstanceState::Terminating,
                None,
                HealthState::TerminatedConfirm
            );
        };
    }

    test_terminating_transition!(
        test_healthy_worker_transitions_to_terminated_confirm,
        HealthState::Healthy { ip: test_ip() }
    );
    test_terminating_transition!(
        test_starting_worker_transitions_to_terminated_confirm,
        HealthState::Starting
    );
    test_terminating_transition!(
        test_retrying_check_worker_transitions_to_terminated_confirm,
        HealthState::RetryingCheck { retrials: 2 }
    );
    test_terminating_transition!(
        test_graceful_shutting_down_worker_transitions_to_terminated_confirm,
        HealthState::GracefulShuttingDown
    );
    test_terminating_transition!(
        test_marked_for_termination_worker_transitions_to_terminated_confirm,
        HealthState::MarkedForTermination
    );
    test_terminating_transition!(
        test_invisible_on_infra_worker_transitions_to_terminated_confirm,
        HealthState::InvisibleOnInfra
    );

    test_state_preserved!(
        test_terminated_confirm_stays_unchanged_when_terminating,
        HealthState::TerminatedConfirm,
        WorkerInstanceState::Terminating,
        None,
        HealthState::TerminatedConfirm
    );

    // ========================================
    // 9. Edge Cases
    // ========================================

    test_transition!(
        test_terminated_confirm_receives_graceful_shutdown_signal,
        HealthState::TerminatedConfirm,
        WorkerInstanceState::Running,
        Some(WorkerHealthKind::GracefulShuttingDown),
        HealthState::GracefulShuttingDown
    );

    #[test]
    fn test_terminating_priority_over_healthy_response() {
        // Case 1: Terminating status from Infra should override Good health response
        let fixture = TestFixture::new();
        let start_time = fixture.start_time();

        fixture
            .with_record(
                "worker1",
                HealthState::Healthy { ip: test_ip() },
                TimeDelta::minutes(-1),
            )
            .with_response(
                "worker1",
                WorkerInstanceState::Terminating,
                Some(WorkerHealthKind::Good),
            )
            .run()
            .assert_state_matches("worker1", |s| matches!(s, HealthState::TerminatedConfirm))
            .assert_transited_at("worker1", start_time);
    }

    #[test]
    fn test_graceful_shutdown_cannot_recover_to_healthy() {
        // Case 2: Once GracefulShuttingDown, it should NEVER go back to Healthy
        let fixture = TestFixture::new();
        let previous_time = fixture.previous_time(2); // 2 minutes ago

        fixture
            .with_record(
                "worker1",
                HealthState::GracefulShuttingDown,
                TimeDelta::minutes(-2),
            )
            .with_response(
                "worker1",
                WorkerInstanceState::Running,
                Some(WorkerHealthKind::Good),
            )
            .run()
            .assert_state_matches("worker1", |s| {
                matches!(s, HealthState::GracefulShuttingDown)
            })
            // The logic we implemented preserves the state but effectively does "nothing" in the match arm,
            // so the timestamp should remain unchanged (from 2 mins ago).
            .assert_transited_at("worker1", previous_time);
    }
}
