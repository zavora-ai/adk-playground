use adk_core::{SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

// ── Amazon Bedrock ──
// Access Claude, Llama, Mistral and other models via AWS Bedrock's Converse API.
// Uses IAM/STS authentication from the standard AWS credential chain —
// no API key needed, just AWS_REGION and valid credentials.

#[derive(JsonSchema, Serialize, Deserialize)]
struct ArchReviewArgs {
    /// Description of the system to review
    system: String,
    /// Expected requests per second
    rps: u32,
}

#[derive(JsonSchema, Serialize, Deserialize)]
struct ThreatArgs {
    /// Architecture components to analyze for threats
    components: Vec<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    let region = std::env::var("AWS_REGION")
        .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
        .unwrap_or_else(|_| "us-east-1".into());
    let model_id = std::env::var("BEDROCK_MODEL_ID")
        .unwrap_or_else(|_| "us.anthropic.claude-sonnet-4-6".into());

    // Bedrock uses IAM credentials from the standard AWS chain — no API key needed
    let config = BedrockConfig::new(&region, &model_id);
    let model = Arc::new(BedrockClient::new(config).await?);

    let review_tool = FunctionTool::new(
        "review_architecture",
        "Review a cloud architecture and return component recommendations with cost estimate",
        |_ctx, args| async move {
            let system = args.get("system").and_then(|v| v.as_str()).unwrap_or("unknown");
            let rps = args.get("rps").and_then(|v| v.as_u64()).unwrap_or(100) as u32;
            let tier = if rps > 10000 { "enterprise" } else if rps > 1000 { "growth" } else { "starter" };

            let mut components = vec![
                serde_json::json!({"service": "API Gateway", "purpose": "Request routing & throttling"}),
                serde_json::json!({"service": "Lambda / ECS", "purpose": "Compute layer"}),
                serde_json::json!({"service": "DynamoDB", "purpose": "Low-latency data store"}),
            ];
            if rps > 1000 {
                components.push(serde_json::json!({"service": "CloudFront", "purpose": "CDN edge caching"}));
                components.push(serde_json::json!({"service": "SQS", "purpose": "Async message queue"}));
                components.push(serde_json::json!({"service": "ElastiCache", "purpose": "In-memory caching layer"}));
            }

            Ok(serde_json::json!({
                "system": system,
                "tier": tier,
                "expected_rps": rps,
                "components": components,
                "estimated_monthly_cost": match tier {
                    "enterprise" => "$5,000-$20,000",
                    "growth" => "$500-$5,000",
                    _ => "$50-$500",
                }
            }))
        },
    )
    .with_parameters_schema::<ArchReviewArgs>();

    let threat_tool = FunctionTool::new(
        "analyze_threats",
        "Perform a threat model analysis on the given architecture components",
        |_ctx, args| async move {
            let components: Vec<String> = args.get("components")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();

            let threats: Vec<_> = components.iter().map(|c| {
                let (threat, mitigation) = match c.to_lowercase() {
                    s if s.contains("api") => ("Unauthorized access / DDoS", "WAF + rate limiting + API keys"),
                    s if s.contains("lambda") || s.contains("ecs") => ("Code injection", "IAM least-privilege + VPC isolation"),
                    s if s.contains("dynamo") || s.contains("rds") => ("Data exfiltration", "Encryption at rest + VPC endpoints"),
                    s if s.contains("sqs") => ("Message tampering", "SSE-SQS encryption + dead-letter queues"),
                    _ => ("Misconfiguration", "Security review + AWS Config rules"),
                };
                serde_json::json!({ "component": c, "threat": threat, "mitigation": mitigation })
            }).collect();

            Ok(serde_json::json!({
                "components_analyzed": components.len(),
                "threats": threats,
            }))
        },
    )
    .with_parameters_schema::<ThreatArgs>();

    let agent = Arc::new(
        LlmAgentBuilder::new("cloud_architect")
            .instruction(
                "You are a cloud solutions architect powered by Amazon Bedrock.\n\
                 Use review_architecture to create infrastructure recommendations,\n\
                 then use analyze_threats to assess security risks of the proposed components.\n\
                 Present a clear architecture overview followed by the security analysis.",
            )
            .model(model)
            .tool(Arc::new(review_tool))
            .tool(Arc::new(threat_tool))
            .build()?,
    );

    let sessions = Arc::new(InMemorySessionService::new());
    sessions
        .create(CreateRequest {
            app_name: "playground".into(),
            user_id: "user".into(),
            session_id: Some("s1".into()),
            state: HashMap::new(),
        })
        .await?;

    let runner = Runner::new(RunnerConfig {
        app_name: "playground".into(),
        agent,
        session_service: sessions,
        artifact_service: None,
        memory_service: None,
        plugin_manager: None,
        run_config: None,
        compaction_config: None,
        context_cache_config: None,
        cache_capable: None,
        request_context: None,
        cancellation_token: None,
    })?;

    println!("🏗️  Amazon Bedrock — {} ({})\n", model_id, region);

    let message = Content::new("user").with_text(
        "Design a high-availability e-commerce API handling 5000 requests/second, \
             then analyze the security threats for the proposed architecture.",
    );
    let mut stream = runner
        .run(UserId::new("user")?, SessionId::new("s1")?, message)
        .await?;

    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                if let Some(text) = part.text() {
                    print!("{}", text);
                }
            }
        }
    }
    println!();
    Ok(())
}
