use adk_action::*;
use serde_json::json;
use std::collections::HashMap;

// ── Action Nodes: HTTP & Merge ──
//
// Demonstrates integration action nodes:
//   1. HTTP — make API calls with auth, headers, rate limiting
//   2. Merge — combine results from parallel branches
//   3. Wait — pause execution (fixed delay, condition polling)
//   4. Error handling — retry, fallback, and stop modes

const THIN: &str = "────────────────────────────────────────────────────────";
const THICK: &str = "════════════════════════════════════════════════════════";

fn main() -> anyhow::Result<()> {
    println!("{THICK}");
    println!("  🌐 Action Nodes: HTTP, Merge & Wait");
    println!("  API calls, parallel merge, delays, error handling");
    println!("{THICK}");

    // ── 1. HTTP Node — API request ──
    println!("\n  ┌─ 1. HTTP NODE — API request with auth");

    let http_get = HttpNodeConfig {
        standard: make_standard_with_retry("fetch_users", "Fetch Users", 3),
        method: HttpMethod::Get,
        url: "https://api.example.com/users?active=true".into(),
        auth: HttpAuth::Bearer(BearerAuth {
            token: "{{secrets.api_token}}".into(),
        }),
        headers: HashMap::from([
            ("Accept".into(), "application/json".into()),
            ("X-Request-ID".into(), "{{request_id}}".into()),
        ]),
        body: HttpBody::None,
        response: HttpResponse {
            response_type: "json".into(),
            status_validation: Some("2xx".into()),
        },
        rate_limit: Some(RateLimit {
            max_requests: 100,
            window_ms: 60000,
        }),
    };

    println!("  │  {} {}", format!("{:?}", http_get.method), http_get.url);
    println!("  │  Auth: Bearer (token from secrets)");
    println!("  │  Headers: {} custom", http_get.headers.len());
    if let Some(rl) = &http_get.rate_limit {
        println!("  │  Rate limit: {} req / {}ms", rl.max_requests, rl.window_ms);
    }
    println!("  │  Retry: {:?}", http_get.standard.error_handling.mode);
    println!("  └─ ✅ GET request configured\n");

    // ── 2. HTTP POST with JSON body ──
    println!("  ┌─ 2. HTTP POST — Send JSON payload");

    let http_post = HttpNodeConfig {
        standard: make_standard("create_order", "Create Order"),
        method: HttpMethod::Post,
        url: "https://api.example.com/orders".into(),
        auth: HttpAuth::ApiKey(ApiKeyAuth {
            header: "X-API-Key".into(),
            value: "{{secrets.order_api_key}}".into(),
        }),
        headers: HashMap::from([("Content-Type".into(), "application/json".into())]),
        body: HttpBody::Json {
            data: json!({
                "customer_id": "{{customer.id}}",
                "items": "{{cart.items}}",
                "total": "{{cart.total}}"
            }),
        },
        response: HttpResponse {
            response_type: "json".into(),
            status_validation: Some("201".into()),
        },
        rate_limit: None,
    };

    println!("  │  {} {}", format!("{:?}", http_post.method), http_post.url);
    println!("  │  Auth: API Key header");
    if let HttpBody::Json { data } = &http_post.body {
        println!("  │  Body: {}", serde_json::to_string(data)?);
    }
    println!("  └─ ✅ POST request configured\n");

    // ── 3. Merge Node — combine parallel results ──
    println!("  ┌─ 3. MERGE NODE — Combine parallel branches");

    let merge = MergeNodeConfig {
        standard: make_standard("combine_results", "Combine Search Results"),
        mode: MergeMode::WaitAll,
        required_count: None,
        combine_strategy: CombineStrategy::Array,
        timeout: Some(MergeTimeout {
            timeout_ms: 10000,
            on_timeout: "partial".into(),
        }),
    };

    println!("  │  Mode: {:?} — wait for all branches", merge.mode);
    println!("  │  Strategy: {:?} — collect into array", merge.combine_strategy);
    if let Some(t) = &merge.timeout {
        println!("  │  Timeout: {}ms → {}", t.timeout_ms, t.on_timeout);
    }

    // Simulate merge
    let branch_results = vec![
        json!({"source": "web", "results": 42}),
        json!({"source": "db", "results": 18}),
        json!({"source": "cache", "results": 7}),
    ];
    println!("  │");
    for r in &branch_results {
        println!(
            "  │  ← {} returned {} results",
            r["source"].as_str().unwrap_or("?"),
            r["results"]
        );
    }
    let total: i64 = branch_results
        .iter()
        .filter_map(|r| r["results"].as_i64())
        .sum();
    println!("  │  → Merged: {} total results from {} branches", total, branch_results.len());
    println!("  └─ ✅ All branches merged\n");

    // ── 4. Wait Node — delay execution ──
    println!("  ┌─ 4. WAIT NODE — Pause execution");

    let wait_fixed = WaitNodeConfig {
        standard: make_standard("rate_limit_pause", "Rate Limit Pause"),
        wait_type: WaitType::Fixed,
        fixed: Some(FixedDuration {
            duration: 2000,
            unit: "ms".into(),
        }),
        until: None,
        webhook: None,
        condition: None,
    };

    let wait_condition = WaitNodeConfig {
        standard: make_standard("poll_deploy", "Poll Deployment"),
        wait_type: WaitType::Condition,
        fixed: None,
        until: None,
        webhook: None,
        condition: Some(ConditionPolling {
            condition: "deploy.status == 'complete'".into(),
            interval_ms: 5000,
            max_wait_ms: 300000,
        }),
    };

    if let Some(f) = &wait_fixed.fixed {
        println!("  │  Fixed: {}{}", f.duration, f.unit);
    }
    if let Some(c) = &wait_condition.condition {
        println!("  │  Poll: \"{}\" every {}ms (max {}ms)",
            c.condition, c.interval_ms, c.max_wait_ms);
    }
    println!("  └─ ✅ Wait nodes configured\n");

    // ── 5. Error Handling modes ──
    println!("  ┌─ 5. ERROR HANDLING — Retry, fallback, stop");

    let modes = [
        ("Stop", ErrorHandling {
            mode: ErrorMode::Stop,
            retry_count: None,
            retry_delay: None,
            fallback_value: None,
        }),
        ("Retry", ErrorHandling {
            mode: ErrorMode::Retry,
            retry_count: Some(3),
            retry_delay: Some(1000),
            fallback_value: None,
        }),
        ("Fallback", ErrorHandling {
            mode: ErrorMode::Fallback,
            retry_count: None,
            retry_delay: None,
            fallback_value: Some(json!({"status": "default", "data": []})),
        }),
        ("Continue", ErrorHandling {
            mode: ErrorMode::Continue,
            retry_count: None,
            retry_delay: None,
            fallback_value: None,
        }),
    ];

    for (label, eh) in &modes {
        let detail = match eh.mode {
            ErrorMode::Stop => "halt workflow on error".into(),
            ErrorMode::Retry => format!(
                "retry {}x with {}ms delay",
                eh.retry_count.unwrap_or(0),
                eh.retry_delay.unwrap_or(0)
            ),
            ErrorMode::Fallback => format!(
                "use default: {}",
                eh.fallback_value.as_ref().map(|v| v.to_string()).unwrap_or_default()
            ),
            ErrorMode::Continue => "skip error, continue workflow".into(),
        };
        println!("  │  {label:10} → {detail}");
    }
    println!("  └─ ✅ All error modes available\n");

    // ── Summary ──
    println!("{THICK}");
    println!("  ✅ HTTP, Merge & Wait action nodes demonstrated");
    println!("  • HTTP supports GET/POST with Bearer, Basic, API Key auth");
    println!("  • Merge combines parallel branches (WaitAll/WaitAny/WaitN)");
    println!("  • Wait pauses with fixed delay or condition polling");
    println!("  • Error handling: stop, retry, fallback, or continue");
    println!("{THICK}");
    Ok(())
}

fn make_standard(id: &str, name: &str) -> StandardProperties {
    StandardProperties {
        id: id.into(),
        name: name.into(),
        description: None,
        position: None,
        error_handling: ErrorHandling {
            mode: ErrorMode::Stop,
            retry_count: None,
            retry_delay: None,
            fallback_value: None,
        },
        tracing: Tracing {
            enabled: true,
            log_level: LogLevel::Info,
        },
        callbacks: Callbacks::default(),
        execution: ExecutionControl {
            timeout: 30000,
            condition: None,
        },
        mapping: InputOutputMapping {
            input_mapping: None,
            output_key: format!("{id}_output"),
        },
    }
}

fn make_standard_with_retry(id: &str, name: &str, retries: u32) -> StandardProperties {
    StandardProperties {
        error_handling: ErrorHandling {
            mode: ErrorMode::Retry,
            retry_count: Some(retries),
            retry_delay: Some(1000),
            fallback_value: None,
        },
        ..make_standard(id, name)
    }
}
