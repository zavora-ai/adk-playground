//! Sandbox policy & capabilities — validates the truthful capability model
//!
//! Demonstrates: BackendCapabilities, validate_policy, validate_request,
//! and the gap between requested and enforced constraints.

use adk_code::{
    BackendCapabilities, ExecutionIsolation, ExecutionLanguage, ExecutionPayload,
    ExecutionRequest, SandboxPolicy, validate_policy, validate_request,
};

fn main() {
    println!("=== Sandbox Policy & Capabilities ===\n");

    // 1. Define what a backend can actually enforce
    let capabilities = BackendCapabilities {
        isolation: ExecutionIsolation::HostLocal,
        enforce_network_policy: false,
        enforce_filesystem_policy: false,
        enforce_environment_policy: true,
        enforce_timeout: true,
        supports_structured_output: true,
        supports_process_execution: true,
        supports_persistent_workspace: false,
        supports_interactive_sessions: false,
    };
    println!("✓ Backend capabilities defined:");
    println!("  isolation: {:?}", capabilities.isolation);
    println!("  timeout: {}, network: {}, filesystem: {}",
        capabilities.enforce_timeout,
        capabilities.enforce_network_policy,
        capabilities.enforce_filesystem_policy,
    );

    // 2. Validate a host-local policy against capabilities
    // host_local() uses NetworkPolicy::Enabled, so no enforcement needed
    let host_policy = SandboxPolicy::host_local();
    let result = validate_policy(&capabilities, &host_policy);
    assert!(result.is_ok());
    println!("✓ host_local policy passes validation against HostLocal backend");

    // 3. strict_rust() requests NetworkPolicy::Disabled — backend can't enforce
    let strict = SandboxPolicy::strict_rust();
    let result = validate_policy(&capabilities, &strict);
    assert!(result.is_err());
    println!("✓ strict_rust policy FAILS against HostLocal (can't enforce network isolation)");

    // 4. Full-capability backend (e.g., container-based)
    let container_caps = BackendCapabilities {
        isolation: ExecutionIsolation::ContainerEphemeral,
        enforce_network_policy: true,
        enforce_filesystem_policy: true,
        enforce_environment_policy: true,
        enforce_timeout: true,
        supports_structured_output: true,
        supports_process_execution: false,
        supports_persistent_workspace: false,
        supports_interactive_sessions: false,
    };
    let result = validate_policy(&container_caps, &strict);
    assert!(result.is_ok());
    println!("✓ strict_rust policy PASSES against ContainerEphemeral backend");

    // 5. validate_request — full request validation
    let request = ExecutionRequest {
        language: ExecutionLanguage::Rust,
        payload: ExecutionPayload::Source {
            code: "fn run(input: serde_json::Value) -> serde_json::Value { input }".to_string(),
        },
        argv: vec![],
        stdin: None,
        input: None,
        sandbox: SandboxPolicy::host_local(),
        identity: None,
    };
    let supported = [ExecutionLanguage::Rust, ExecutionLanguage::Command];
    let result = validate_request(&capabilities, &supported, &request);
    assert!(result.is_ok());
    println!("✓ validate_request passes for Rust on HostLocal with host_local policy");

    // 6. Unsupported language rejection
    let py_request = ExecutionRequest {
        language: ExecutionLanguage::Python,
        payload: ExecutionPayload::Source { code: "print('hi')".to_string() },
        argv: vec![],
        stdin: None,
        input: None,
        sandbox: SandboxPolicy::host_local(),
        identity: None,
    };
    let result = validate_request(&capabilities, &supported, &py_request);
    assert!(result.is_err());
    println!("✓ validate_request rejects unsupported language (Python)");

    // 7. Isolation class comparison
    assert_ne!(ExecutionIsolation::HostLocal, ExecutionIsolation::ContainerEphemeral);
    assert_ne!(ExecutionIsolation::InProcess, ExecutionIsolation::HostLocal);
    println!("✓ Isolation classes are distinct and comparable");

    println!("\nIsolation classes:");
    println!("  InProcess          — embedded engine (e.g., JS)");
    println!("  HostLocal          — child process, no OS isolation");
    println!("  ContainerEphemeral — destroyed after execution");
    println!("  ContainerPersistent — survives across requests");
    println!("  ProviderHosted     — remote execution service");

    println!("\n=== All sandbox tests passed! ===");
}
