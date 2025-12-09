use oci_rust_sdk::compute::{LifecycleState, ListInstancesRequest, ListInstancesRequestRequiredFields};

fn main() {
    println!("=== Testing LifecycleState Enum ===\n");

    // Test 1: Display trait implementation
    println!("Test 1: LifecycleState Display trait");
    let states = vec![
        (LifecycleState::Moving, "MOVING"),
        (LifecycleState::Provisioning, "PROVISIONING"),
        (LifecycleState::Running, "RUNNING"),
        (LifecycleState::Starting, "STARTING"),
        (LifecycleState::Stopping, "STOPPING"),
        (LifecycleState::Stopped, "STOPPED"),
        (LifecycleState::CreatingImage, "CREATING_IMAGE"),
        (LifecycleState::Terminating, "TERMINATING"),
        (LifecycleState::Terminated, "TERMINATED"),
    ];

    for (state, expected) in &states {
        let result = state.to_string();
        assert_eq!(
            &result, expected,
            "Failed: {:?} should be {}",
            state, expected
        );
        println!("  ✓ {:?} -> \"{}\"", state, result);
    }

    // Test 2: Usage in ListInstancesRequest
    println!("\nTest 2: LifecycleState in ListInstancesRequest");
    let test_cases = vec![
        (LifecycleState::Running, "RUNNING"),
        (LifecycleState::Stopped, "STOPPED"),
        (LifecycleState::Terminated, "TERMINATED"),
    ];

    for (state, expected) in &test_cases {
        let request = ListInstancesRequest::builder(ListInstancesRequestRequiredFields {
            compartment_id: "test-compartment-id".to_string(),
        })
            .lifecycle_state(*state)
            .build();

        let params = request.to_query_params();
        let lifecycle_param = params
            .iter()
            .find(|(k, _)| k == "lifecycleState")
            .map(|(_, v)| v.as_str());

        assert_eq!(
            lifecycle_param,
            Some(*expected),
            "Failed: {:?} should produce lifecycleState={}",
            state,
            expected
        );
        println!("  ✓ {:?} -> lifecycleState={}", state, expected);
    }

    println!("\nTest 3: Combined filters with LifecycleState");
    let request = ListInstancesRequest::builder(ListInstancesRequestRequiredFields {
        compartment_id: "compartment-123".to_string(),
    })
        .lifecycle_state(LifecycleState::Running)
        .display_name("my-instance")
        .limit(10)
        .build();

    let params = request.to_query_params();

    println!("  Query parameters:");
    for (key, value) in &params {
        println!("    {} = {}", key, value);
    }

    // Verify lifecycle state is correctly converted
    let lifecycle_value = params
        .iter()
        .find(|(k, _)| k == "lifecycleState")
        .map(|(_, v)| v.as_str());

    assert_eq!(lifecycle_value, Some("RUNNING"));
    println!("  ✓ lifecycleState correctly set to RUNNING");

    println!("\n✅ All tests passed!");
    println!("\nConclusion:");
    println!("  - LifecycleState enum provides type safety");
    println!("  - API requests use string values (MOVING, RUNNING, etc.)");
    println!("  - Display trait enables automatic .to_string() conversion");
}
