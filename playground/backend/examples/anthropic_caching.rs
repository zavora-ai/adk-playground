use adk_core::{SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use std::collections::HashMap;
use std::sync::Arc;

// ── Anthropic Prompt Caching — Multi-Turn Agent ──
// Demonstrates `.with_prompt_caching(true)` on AnthropicConfig.
// The agent has a large system prompt (architecture reference doc).
// Turn 1 creates the cache (25% surcharge), Turn 2 hits it (90% discount).
// Watch the token usage metadata to see cache_read vs cache_creation counts.

const REFERENCE_DOC: &str = r#"
# Software Architecture Patterns Reference

## 1. Microservices Architecture
Microservices decompose applications into small, independently deployable services.
Each service owns its data, communicates via APIs, and can be scaled independently.
Key principles: single responsibility, loose coupling, independent deployment,
decentralized data management, infrastructure automation, design for failure.
Benefits: independent scaling, technology diversity, fault isolation, team autonomy.
Challenges: distributed system complexity, data consistency, network latency.
Common patterns: API Gateway, Service Discovery, Circuit Breaker, Saga Pattern,
Event Sourcing, CQRS, Sidecar, Ambassador, Strangler Fig.

## 2. Event-Driven Architecture
Systems communicate through events — immutable records of state changes.
Producers emit events without knowing consumers. Enables loose coupling.
Components: Event Producers, Event Channels, Event Consumers, Event Store.
Technologies: Apache Kafka, RabbitMQ, AWS EventBridge, NATS, Redis Streams.

## 3. Domain-Driven Design (DDD)
Aligns software design with business domains. Core concepts: Bounded Contexts,
Aggregates, Entities, Value Objects, Domain Events, Repositories, Services.
Strategic patterns: Context Mapping, Shared Kernel, Anti-Corruption Layer.
Tactical patterns: Aggregate Root, Domain Service, Factory, Specification.

## 4. Clean Architecture
Dependency rule: dependencies point inward. Layers from inside out:
Entities, Use Cases, Interface Adapters, Frameworks & Drivers.
Benefits: testability, independence from frameworks/UI/database, flexibility.
Related: Hexagonal Architecture (Ports & Adapters), Onion Architecture.

## 5. Reactive Systems
Responsive, Resilient, Elastic, Message-Driven (Reactive Manifesto).
Patterns: Actor Model, Reactive Streams, Backpressure, Circuit Breaker.
Technologies: Akka, Project Reactor, RxJava, Vert.x, Quarkus.

## 6. Serverless Architecture
Functions as a Service (FaaS) — stateless containers triggered by events.
Patterns: Function Composition, Fan-out/Fan-in, Async Messaging.
Considerations: cold starts, execution limits, vendor lock-in, cost at scale.

## 7. CQRS and Event Sourcing
Separates read and write models. Stores state as event sequence.
Benefits: optimized models, audit trail, temporal queries, event replay.

## 8. Service Mesh
Infrastructure layer for service-to-service communication.
Components: Data Plane (sidecar proxies), Control Plane (configuration).
Technologies: Istio, Linkerd, Consul Connect, AWS App Mesh.
"#;


#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key =
        std::env::var("ANTHROPIC_API_KEY").expect("Set ANTHROPIC_API_KEY in your .env file");

    // Enable prompt caching — Anthropic caches system instructions automatically
    let model = Arc::new(AnthropicClient::new(
        AnthropicConfig::new(&api_key, "claude-sonnet-4-20250514")
            .with_prompt_caching(true)
            .with_max_tokens(1024),
    )?);

    let system_instruction = format!(
        "You are a software architecture expert. Answer questions using ONLY the \
         reference material below. Cite the section number in your answer.\n\n{REFERENCE_DOC}"
    );

    let agent = Arc::new(
        LlmAgentBuilder::new("architecture_expert")
            .instruction(&system_instruction)
            .model(model)
            .build()?,
    );

    let sessions = Arc::new(InMemorySessionService::new());
    let uid = UserId::new("user")?;
    let sid = SessionId::new("s1")?;
    sessions
        .create(CreateRequest {
            app_name: "playground".into(),
            user_id: uid.to_string(),
            session_id: Some(sid.to_string()),
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

    println!("=== Anthropic Prompt Caching — Multi-Turn Agent ===");
    println!("System prompt: ~2K chars of architecture reference material");
    println!("with_prompt_caching(true) → cache_control on system instructions\n");

    // ── Turn 1: Cache Creation ──
    println!("── Turn 1: Cache Creation (25% surcharge on first use) ──\n");
    let msg1 = Content::new("user")
        .with_text("What is the dependency rule in Clean Architecture? Cite the section.");
    let mut stream = runner.run(uid.clone(), sid.clone(), msg1).await?;
    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                if let Some(text) = part.text() {
                    print!("{}", text);
                }
            }
        }
        if let Some(usage) = &event.llm_response.usage_metadata {
            if usage.total_token_count > 0 {
                println!("\n\n📊 Turn 1 tokens — input: {}, output: {}", usage.prompt_token_count, usage.candidates_token_count);
                if let Some(cache_create) = usage.cache_creation_input_token_count {
                    println!("   cache_creation: {} ← system prompt cached (25% surcharge)", cache_create);
                }
                if let Some(cache_read) = usage.cache_read_input_token_count {
                    println!("   cache_read: {}", cache_read);
                }
            }
        }
    }

    // ── Turn 2: Cache Hit ──
    println!("\n── Turn 2: Cache Hit (90% discount on cached tokens) ──\n");
    let msg2 = Content::new("user")
        .with_text("Compare microservices and event-driven architecture. Cite sections.");
    let mut stream = runner.run(uid.clone(), sid.clone(), msg2).await?;
    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                if let Some(text) = part.text() {
                    print!("{}", text);
                }
            }
        }
        if let Some(usage) = &event.llm_response.usage_metadata {
            if usage.total_token_count > 0 {
                println!("\n\n📊 Turn 2 tokens — input: {}, output: {}", usage.prompt_token_count, usage.candidates_token_count);
                if let Some(cache_read) = usage.cache_read_input_token_count {
                    println!("   cache_read: {} ← 90% cheaper! System prompt served from cache", cache_read);
                }
                if let Some(cache_create) = usage.cache_creation_input_token_count {
                    println!("   cache_creation: {}", cache_create);
                }
            }
        }
    }

    println!("\n\n=== Key Takeaways ===");
    println!("• with_prompt_caching(true) adds cache_control to system instructions");
    println!("• First request: cache_creation tokens (25% surcharge)");
    println!("• Subsequent requests: cache_read tokens (90% discount)");
    println!("• Ideal for agents with large system prompts or reference documents");
    Ok(())
}
