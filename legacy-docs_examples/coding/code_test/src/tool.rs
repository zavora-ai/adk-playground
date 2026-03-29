//! CodeTool — validates the adk_core::Tool implementation for code execution
//!
//! Demonstrates: CodeTool construction, schema inspection, and the
//! structured error-as-information pattern.

use adk_code::{CodeTool, RustExecutor, RustExecutorConfig};
use adk_core::Tool;
use adk_sandbox::SandboxBackend;
use adk_sandbox::backend::{BackendCapabilities, EnforcedLimits};
use adk_sandbox::error::SandboxError;
use adk_sandbox::types::{ExecRequest, ExecResult, Language};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

// Minimal mock backend for compilation validation
struct MockBackend;

#[async_trait]
impl SandboxBackend for MockBackend {
    fn name(&self) -> &str { "mock" }
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            supported_languages: vec![Language::Command],
            isolation_class: "mock".to_string(),
            enforced_limits: EnforcedLimits {
                timeout: true, memory: false,
                network_isolation: false, filesystem_isolation: false,
                environment_isolation: false,
            },
        }
    }
    async fn execute(&self, _req: ExecRequest) -> Result<ExecResult, SandboxError> {
        Ok(ExecResult {
            stdout: r#"{"result":"ok"}"#.to_string(),
            stderr: String::new(),
            exit_code: 0,
            duration: Duration::from_millis(5),
        })
    }
}

fn main() {
    println!("=== CodeTool Validation ===\n");

    // 1. Construct CodeTool from executor + backend
    let backend = Arc::new(MockBackend);
    let executor = RustExecutor::new(backend, RustExecutorConfig::default());
    let tool = CodeTool::new(executor);

    // 2. Tool metadata
    assert_eq!(tool.name(), "code_exec");
    assert!(tool.description().contains("Rust"));
    assert!(tool.description().contains("fn run"));
    println!("✓ name='code_exec', description mentions Rust entry point");

    // 3. Required scopes for authorization
    let scopes = tool.required_scopes();
    assert!(scopes.contains(&"code:execute"));
    assert!(scopes.contains(&"code:execute:rust"));
    println!("✓ Required scopes: {:?}", scopes);

    // 4. Parameters schema validation
    let schema = tool.parameters_schema().unwrap();
    assert_eq!(schema["type"], "object");

    // Check all expected properties exist
    let props = schema["properties"].as_object().unwrap();
    assert!(props.contains_key("language"));
    assert!(props.contains_key("code"));
    assert!(props.contains_key("input"));
    assert!(props.contains_key("timeout_secs"));
    println!("✓ Schema has language, code, input, timeout_secs properties");

    // Only 'code' is required
    let required: Vec<&str> = schema["required"]
        .as_array().unwrap()
        .iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(required, vec!["code"]);
    println!("✓ Only 'code' is required (language defaults to 'rust')");

    // Language enum
    let lang_enum = schema["properties"]["language"]["enum"].as_array().unwrap();
    assert!(lang_enum.iter().any(|v| v.as_str() == Some("rust")));
    println!("✓ Language enum includes 'rust'");

    // Timeout bounds
    let timeout = &schema["properties"]["timeout_secs"];
    assert_eq!(timeout["default"], 30);
    assert_eq!(timeout["minimum"], 1);
    assert_eq!(timeout["maximum"], 300);
    println!("✓ Timeout: default=30s, min=1s, max=300s");

    // 5. RustExecutorConfig defaults
    let config = RustExecutorConfig::default();
    println!("✓ RustExecutorConfig::default() compiles");
    let _ = format!("{:?}", config);
    println!("✓ RustExecutorConfig implements Debug");

    println!("\n=== All CodeTool tests passed! ===");
}
