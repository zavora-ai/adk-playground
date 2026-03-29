//! Code execution types — validates ExecutionRequest, SandboxPolicy, and related types
//!
//! Demonstrates: ExecutionLanguage, ExecutionPayload, SandboxPolicy,
//! ExecutionRequest construction, and policy presets.

use adk_code::{
    ExecutionLanguage, ExecutionPayload, ExecutionRequest, ExecutionStatus,
    NetworkPolicy, FilesystemPolicy, EnvironmentPolicy, SandboxPolicy,
};
use std::time::Duration;

fn main() {
    println!("=== Code Execution Types ===\n");

    // 1. ExecutionLanguage variants
    assert_eq!(format!("{}", ExecutionLanguage::Rust), "Rust");
    assert_eq!(format!("{}", ExecutionLanguage::JavaScript), "JavaScript");
    assert_eq!(format!("{}", ExecutionLanguage::Python), "Python");
    assert_eq!(format!("{}", ExecutionLanguage::Wasm), "Wasm");
    assert_eq!(format!("{}", ExecutionLanguage::Command), "Command");
    println!("✓ All 5 ExecutionLanguage variants display correctly");

    // 2. ExecutionPayload::Source
    let payload = ExecutionPayload::Source {
        code: r#"
fn run(input: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "greeting": "hello from sandbox" })
}
"#.to_string(),
    };
    match &payload {
        ExecutionPayload::Source { code } => {
            assert!(code.contains("fn run"));
            println!("✓ Source payload contains entry point");
        }
        _ => panic!("Expected Source payload"),
    }

    // 3. SandboxPolicy::strict_rust — the recommended default
    let strict = SandboxPolicy::strict_rust();
    assert!(matches!(strict.network, NetworkPolicy::Disabled));
    assert!(matches!(strict.filesystem, FilesystemPolicy::None));
    assert!(matches!(strict.environment, EnvironmentPolicy::None));
    println!("✓ strict_rust: no network, no filesystem, no env");
    println!("  timeout={:?}, max_stdout={}B", strict.timeout, strict.max_stdout_bytes);

    // 4. Build a full ExecutionRequest
    let request = ExecutionRequest {
        language: ExecutionLanguage::Rust,
        payload,
        argv: vec!["--verbose".to_string()],
        stdin: None,
        input: Some(serde_json::json!({"name": "world"})),
        sandbox: SandboxPolicy::strict_rust(),
        identity: None,
    };
    assert_eq!(request.language, ExecutionLanguage::Rust);
    assert_eq!(request.argv.len(), 1);
    assert!(request.input.is_some());
    println!("✓ ExecutionRequest constructed with all fields");

    // 5. ExecutionStatus variants
    let statuses = [
        ExecutionStatus::Success,
        ExecutionStatus::CompileFailed,
        ExecutionStatus::Failed,
        ExecutionStatus::Timeout,
        ExecutionStatus::Rejected,
    ];
    for status in &statuses {
        let debug = format!("{:?}", status);
        assert!(!debug.is_empty());
    }
    println!("✓ All 5 ExecutionStatus variants are valid");

    // 6. Custom sandbox policy — network enabled, custom timeout
    let custom = SandboxPolicy {
        network: NetworkPolicy::Enabled,
        filesystem: FilesystemPolicy::None,
        environment: EnvironmentPolicy::None,
        timeout: Duration::from_secs(10),
        max_stdout_bytes: 1024 * 64,
        max_stderr_bytes: 1024 * 64,
        working_directory: None,
    };
    assert!(matches!(custom.network, NetworkPolicy::Enabled));
    assert!(matches!(custom.filesystem, FilesystemPolicy::None));
    assert_eq!(custom.timeout, Duration::from_secs(10));
    assert_eq!(custom.max_stdout_bytes, 65536);
    println!("✓ Custom policy: network=enabled, timeout=10s, stdout=64KB");

    // 7. Host-local policy preset
    let host = SandboxPolicy::host_local();
    assert!(matches!(host.network, NetworkPolicy::Enabled));
    println!("✓ host_local: network enabled (host cannot restrict)");

    // 8. Strict JS policy preset
    let js = SandboxPolicy::strict_js();
    assert_eq!(js.timeout, Duration::from_secs(5));
    println!("✓ strict_js: 5s timeout for lightweight transforms");

    // 9. Source validation helper
    let valid = adk_code::validate_rust_source(
        "fn run(input: serde_json::Value) -> serde_json::Value { input }"
    );
    assert!(valid.is_ok());
    println!("✓ validate_rust_source accepts valid entry point");

    let invalid = adk_code::validate_rust_source("fn main() {}");
    assert!(invalid.is_err());
    println!("✓ validate_rust_source rejects fn main (harness provides it)");

    println!("\n=== All type tests passed! ===");
}
