use oci_rust_sdk::core::auth::{AuthProvider, SimpleAuthProvider};
use oci_rust_sdk::core::region::Region;

fn main() {
    println!("SimpleAuthProvider Example\n");

    // Example 1: Using the constructor
    println!("=== Example 1: Using new() constructor ===");
    let provider1 = SimpleAuthProvider::new(
        "ocid1.tenancy.oc1..aaaaaaaexample",
        "ocid1.user.oc1..aaaaaaaexample",
        "aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:99",
        "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA...\n-----END RSA PRIVATE KEY-----",
    );

    println!("Tenancy: {}", provider1.tenancy());
    println!("User: {}", provider1.user());
    println!("Key ID: {}", provider1.get_key_id());
    println!("Region: {:?}\n", provider1.region());

    // Example 2: Using the builder pattern (recommended)
    println!("=== Example 2: Using builder pattern ===");
    let provider2 = SimpleAuthProvider::builder()
        .tenancy("ocid1.tenancy.oc1..aaaaaaaexample")
        .user("ocid1.user.oc1..aaaaaaaexample")
        .fingerprint("aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:99")
        .private_key(
            "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA...\n-----END RSA PRIVATE KEY-----",
        )
        .passphrase("my-secret-passphrase")
        .region(Region::ApSeoul1)
        .auth_type("api_key")
        .build();

    println!("Tenancy: {}", provider2.tenancy());
    println!("User: {}", provider2.user());
    println!("Key ID: {}", provider2.get_key_id());
    println!("Region: {:?}", provider2.region());
    println!("Auth Type: {:?}", provider2.auth_type());
    println!("Has Passphrase: {}\n", provider2.get_passphrase().is_some());

    // Example 3: Setting region after creation
    println!("=== Example 3: Setting region dynamically ===");
    let mut provider3 = SimpleAuthProvider::new(
        "ocid1.tenancy.oc1..aaaaaaaexample",
        "ocid1.user.oc1..aaaaaaaexample",
        "aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:99",
        "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA...\n-----END RSA PRIVATE KEY-----",
    );

    println!("Initial region: {:?}", provider3.region());
    provider3.set_region(Region::UsPhoenix1);
    println!("After set_region: {:?}\n", provider3.region());

    // Example 4: Using with_* methods (fluent API)
    println!("=== Example 4: Fluent API with with_* methods ===");
    let provider4 = SimpleAuthProvider::new(
        "ocid1.tenancy.oc1..aaaaaaaexample",
        "ocid1.user.oc1..aaaaaaaexample",
        "aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:99",
        "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA...\n-----END RSA PRIVATE KEY-----",
    )
    .with_region(Region::EuFrankfurt1)
    .with_auth_type("instance_principal")
    .with_delegation_token("delegation-token-here")
    .with_session_token("session-token-here");

    println!("Region: {:?}", provider4.region());
    println!("Auth Type: {:?}", provider4.auth_type());
    println!("Delegation Token: {:?}", provider4.delegation_token());
    println!("Session Token: {:?}\n", provider4.session_token());

    // Example 5: Error handling with try_build
    println!("=== Example 5: Error handling with try_build ===");
    match SimpleAuthProvider::builder()
        .tenancy("ocid1.tenancy.oc1..aaaaaaaexample")
        .user("ocid1.user.oc1..aaaaaaaexample")
        // Missing fingerprint and private_key
        .try_build()
    {
        Ok(_) => println!("Provider created successfully"),
        Err(e) => println!("Error creating provider: {}", e),
    }

    match SimpleAuthProvider::builder()
        .tenancy("ocid1.tenancy.oc1..aaaaaaaexample")
        .user("ocid1.user.oc1..aaaaaaaexample")
        .fingerprint("aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:99")
        .private_key("-----BEGIN RSA PRIVATE KEY-----\nkey\n-----END RSA PRIVATE KEY-----")
        .try_build()
    {
        Ok(provider) => println!("Provider created successfully: {}", provider.get_key_id()),
        Err(e) => println!("Error creating provider: {}", e),
    }
}
