use adk_core::{Event, Part};

/// Prints a section banner for lifecycle phase separation.
///
/// Outputs a visually distinct separator with the given title centered
/// between box-drawing lines.
pub fn banner(title: &str) {
    println!("\n{}", "═".repeat(60));
    println!("  {title}");
    println!("{}\n", "═".repeat(60));
}

/// Formats and prints an agent event to stdout.
///
/// Displays:
/// - Tool call names and arguments for `FunctionCall` parts
/// - Tool results (stdout, stderr, exit_code, timed_out) for `FunctionResponse` parts
/// - LLM text responses (streaming partial and final)
pub fn print_event(event: &Event) {
    let Some(ref content) = event.llm_response.content else {
        return;
    };
    for part in &content.parts {
        match part {
            Part::FunctionCall { name, args, .. } => {
                println!("  🔧 Tool call: {name}");
                if let Ok(pretty) = serde_json::to_string_pretty(args) {
                    for line in pretty.lines() {
                        println!("     {line}");
                    }
                }
            }
            Part::FunctionResponse { function_response, .. } => {
                print_tool_result(&function_response.response);
            }
            _ => {
                if let Some(text) = part.text()
                    && !text.is_empty()
                {
                    if event.llm_response.partial {
                        print!("{text}");
                    } else {
                        println!("  🤖 Agent: {text}");
                    }
                }
            }
        }
    }
}

/// Formats a tool result JSON value for display.
///
/// For command execution results, displays stdout, stderr, exit_code, and timed_out.
/// For error results, displays the error type and message.
fn print_tool_result(output: &serde_json::Value) {
    // Handle error responses
    if let Some(error) = output.get("error").and_then(|v| v.as_str()) {
        println!("  ❌ Error: {error}");
        if let Some(msg) = output.get("message").and_then(|v| v.as_str()) {
            println!("     {msg}");
        }
        return;
    }

    let exit_code = output.get("exit_code").and_then(|v| v.as_i64());
    let timed_out = output.get("timed_out").and_then(|v| v.as_bool()).unwrap_or(false);

    if timed_out {
        println!("  ⏱️  Command timed out (timed_out: true)");
    } else if let Some(code) = exit_code {
        if code == 0 {
            println!("  ✅ Exit code: 0");
        } else {
            println!("  ❌ Exit code: {code}");
        }
    }

    if let Some(stdout) = output.get("stdout").and_then(|v| v.as_str())
        && !stdout.is_empty()
    {
        println!("  ┌─── stdout ─────────────────────────────────");
        for line in stdout.lines().take(20) {
            println!("  │ {line}");
        }
        println!("  └─────────────────────────────────────────────");
    }

    if let Some(stderr) = output.get("stderr").and_then(|v| v.as_str())
        && !stderr.is_empty()
    {
        println!("  ┌─── stderr ─────────────────────────────────");
        for line in stderr.lines().take(10) {
            println!("  │ {line}");
        }
        println!("  └─────────────────────────────────────────────");
    }
}

/// Prints the final summary showing which lifecycle phases completed.
///
/// Each phase is displayed with a checkmark (✓) for success or cross (✗) for failure.
pub fn print_summary(phases: &[(&str, bool)]) {
    banner("Summary");
    for (phase, success) in phases {
        let icon = if *success { "✓" } else { "✗" };
        println!("  {icon} {phase}");
    }
}
