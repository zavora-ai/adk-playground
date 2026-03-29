use serde::Serialize;
use std::path::Path;

#[derive(Serialize, Clone)]
pub struct Example {
    pub id: String,
    pub name: String,
    pub category: String,
    pub description: String,
    pub code: String,
}

struct ExampleMeta {
    id: &'static str,
    name: &'static str,
    category: &'static str,
    description: &'static str,
    file: &'static str,
}

const REGISTRY: &[ExampleMeta] = &[
    // ── Getting Started ──
    ExampleMeta {
        id: "quickstart",
        name: "Quickstart",
        category: "Getting Started",
        description: "Basic LLM agent with Gemini — the simplest ADK program",
        file: "quickstart.rs",
    },
    ExampleMeta {
        id: "template",
        name: "Instruction Templates",
        category: "Getting Started",
        description: "Dynamic instructions with session state placeholders",
        file: "template.rs",
    },
    ExampleMeta {
        id: "structured_output",
        name: "Structured Output",
        category: "Getting Started",
        description: "Force JSON responses matching a schema",
        file: "structured_output.rs",
    },
    // ── Function Tools ──
    ExampleMeta {
        id: "function_tool",
        name: "Basic Function Tools",
        category: "Function Tools",
        description: "Agent with typed function tools and schema validation",
        file: "function_tool.rs",
    },
    ExampleMeta {
        id: "multi_tools",
        name: "Multiple Tools",
        category: "Function Tools",
        description: "Agent with weather, calculator, and unit converter tools",
        file: "multi_tools.rs",
    },
    ExampleMeta {
        id: "multi_turn",
        name: "Multi-Turn Conversation",
        category: "Function Tools",
        description: "Shopping assistant with cart — tool context preserved across 3 turns",
        file: "multi_turn.rs",
    },
    // ── Agents ──
    ExampleMeta {
        id: "agent_tool",
        name: "Agent-as-Tool",
        category: "Agents",
        description: "Wrap specialist agents as callable tools for a coordinator",
        file: "agent_tool.rs",
    },
    ExampleMeta {
        id: "customer_service",
        name: "Customer Service",
        category: "Agents",
        description: "Billing issue → agent escalation → manager approval — full resolution flow",
        file: "customer_service.rs",
    },
    ExampleMeta {
        id: "conditional_router",
        name: "LLM Conditional Router",
        category: "Agents",
        description: "LLM classifies queries and routes to specialist agents",
        file: "conditional_router.rs",
    },
    // ── Callbacks ──
    ExampleMeta {
        id: "callbacks_logging",
        name: "Logging Callbacks",
        category: "Callbacks",
        description: "Before/after callbacks for logging agent interactions",
        file: "callbacks_logging.rs",
    },
    ExampleMeta {
        id: "callbacks_guardrails",
        name: "Input Guardrails",
        category: "Callbacks",
        description: "Block inappropriate content with before_callback guardrails",
        file: "callbacks_guardrails.rs",
    },
    // ── Workflows ──
    ExampleMeta {
        id: "sequential",
        name: "Sequential Pipeline",
        category: "Workflows",
        description: "Chain agents in a multi-step pipeline (research → write → edit)",
        file: "sequential.rs",
    },
    ExampleMeta {
        id: "parallel",
        name: "Parallel Analysis",
        category: "Workflows",
        description: "Run multiple agents concurrently and merge results",
        file: "parallel.rs",
    },
    ExampleMeta {
        id: "loop_workflow",
        name: "Iterative Loop",
        category: "Workflows",
        description: "Refine content in a loop until quality threshold is met",
        file: "loop_workflow.rs",
    },
    // ── Graph ──
    ExampleMeta {
        id: "graph_workflow",
        name: "Graph Pipeline",
        category: "Graph",
        description: "Analyst → Writer → Editor agents in a sequential graph with deterministic data prep nodes",
        file: "graph_workflow.rs",
    },
    ExampleMeta {
        id: "graph_conditional",
        name: "Conditional Routing",
        category: "Graph",
        description: "LLM classifier routes support tickets to specialist agents via conditional edges",
        file: "graph_conditional.rs",
    },
    ExampleMeta {
        id: "react_pattern",
        name: "ReAct Pattern",
        category: "Graph",
        description: "Iterative reasoning with tools in a graph cycle",
        file: "react_pattern.rs",
    },
    ExampleMeta {
        id: "supervisor_routing",
        name: "Supervisor Routing",
        category: "Graph",
        description: "Supervisor delegates tasks to specialist agent nodes",
        file: "supervisor_routing.rs",
    },
    // ── Sessions & State ──
    ExampleMeta {
        id: "session_state",
        name: "Session & State",
        category: "Sessions",
        description: "Manage conversation sessions with Runner and state",
        file: "session_state.rs",
    },
    ExampleMeta {
        id: "postgres_sessions",
        name: "PostgreSQL Sessions",
        category: "Sessions",
        description: "ACID-compliant session persistence with PostgreSQL — three-tier state, JSONB, advisory-lock migrations",
        file: "postgres_sessions.rs",
    },
    ExampleMeta {
        id: "mongodb_sessions",
        name: "MongoDB Sessions",
        category: "Sessions",
        description: "Schema-flexible document sessions with MongoDB — nested state, arrays, TTL indexes",
        file: "mongodb_sessions.rs",
    },
    ExampleMeta {
        id: "neo4j_sessions",
        name: "Neo4j Sessions",
        category: "Sessions",
        description: "Graph-powered session relationships with Neo4j — nodes, constraints, Cypher queries",
        file: "neo4j_sessions.rs",
    },
    // ── Model Providers ──
    ExampleMeta {
        id: "openai_quickstart",
        name: "OpenAI",
        category: "Providers",
        description: "Responses API — o4-mini reasoning with configurable effort + tool use",
        file: "openai_quickstart.rs",
    },
    ExampleMeta {
        id: "anthropic_quickstart",
        name: "Anthropic",
        category: "Providers",
        description: "Claude Sonnet 4.5 with extended thinking (10K budget) + code review",
        file: "anthropic_quickstart.rs",
    },
    ExampleMeta {
        id: "deepseek_quickstart",
        name: "DeepSeek",
        category: "Providers",
        description: "DeepSeek Reasoner with chain-of-thought for math & logic",
        file: "deepseek_quickstart.rs",
    },
    ExampleMeta {
        id: "mistral_quickstart",
        name: "Mistral",
        category: "Providers",
        description: "Mistral Medium — multilingual translation + sentiment tools",
        file: "mistral_quickstart.rs",
    },
    ExampleMeta {
        id: "xai_quickstart",
        name: "xAI (Grok)",
        category: "Providers",
        description: "Grok-3-mini-fast debugging assistant with tool use",
        file: "xai_quickstart.rs",
    },
    ExampleMeta {
        id: "azure_ai_quickstart",
        name: "Azure AI",
        category: "Providers",
        description: "Azure AI Inference endpoint — text classification + summarization with Llama/Mistral",
        file: "azure_ai_quickstart.rs",
    },
    ExampleMeta {
        id: "bedrock_quickstart",
        name: "AWS Bedrock",
        category: "Providers",
        description: "Amazon Bedrock with Claude — cloud architecture design + threat modeling via IAM auth",
        file: "bedrock_quickstart.rs",
    },
    ExampleMeta {
        id: "openrouter_quickstart",
        name: "OpenRouter",
        category: "Providers",
        description: "Multi-provider AI gateway — 200+ models with tool use, provider routing, and automatic fallback",
        file: "openrouter_quickstart.rs",
    },
    // ── Audio ──
    ExampleMeta {
        id: "poem_tts",
        name: "Poem → Speech",
        category: "Audio",
        description: "LLM writes a random poem, Gemini TTS synthesizes it to audio",
        file: "poem_tts.rs",
    },
    ExampleMeta {
        id: "realtime_audio",
        name: "Realtime Voice",
        category: "Audio",
        description: "OpenAI Realtime API — text prompt to expressive voice audio via WebSocket",
        file: "realtime_audio.rs",
    },
    ExampleMeta {
        id: "realtime_session_update",
        name: "Realtime Session Update",
        category: "Audio",
        description: "Mid-session persona switch — general assistant → travel agent with swapped tools, no reconnect",
        file: "realtime_session_update.rs",
    },
    ExampleMeta {
        id: "realtime_tools",
        name: "Realtime Tools",
        category: "Audio",
        description: "Function calling in voice — weather, calculator, and time tools over a single WebSocket",
        file: "realtime_tools.rs",
    },
    ExampleMeta {
        id: "gemini_live_tools",
        name: "Gemini Live Tools",
        category: "Audio",
        description: "Gemini Live voice agent with weather + time tools — native audio, tool call → response loop",
        file: "gemini_live_tools.rs",
    },
    ExampleMeta {
        id: "gemini_live_context",
        name: "Gemini Live Context Switch",
        category: "Audio",
        description: "Mid-session persona switch via session resumption — tech support → billing agent with swapped tools",
        file: "gemini_live_context.rs",
    },
    // ── Extensions ──
    ExampleMeta {
        id: "skill_discovery",
        name: "Skill Discovery",
        category: "Extensions",
        description: "Discover, parse, score, and inject agentskills.io skill files into prompts",
        file: "skill_discovery.rs",
    },
    ExampleMeta {
        id: "plugin_system",
        name: "Plugin System",
        category: "Extensions",
        description: "Lifecycle hooks for agents — message, model, tool, and run callbacks",
        file: "plugin_system.rs",
    },
    // ── Coding ──
    ExampleMeta {
        id: "code_execution",
        name: "Code Execution",
        category: "Coding",
        description: "Typed sandbox with truthful capability model — policy validation and CodeTool",
        file: "code_execution.rs",
    },
    ExampleMeta {
        id: "cli_launcher",
        name: "CLI Launcher",
        category: "Coding",
        description: "Deploy agents as interactive REPL or HTTP server with streaming",
        file: "cli_launcher.rs",
    },
    // ── RAG ──
    ExampleMeta {
        id: "rag_multi_collection",
        name: "Multi-Collection RAG",
        category: "RAG",
        description: "Domain-isolated knowledge bases with cross-collection search",
        file: "rag_multi_collection.rs",
    },
    ExampleMeta {
        id: "rag_custom_embedder",
        name: "Custom Embedder",
        category: "RAG",
        description: "Implement EmbeddingProvider trait — TF-IDF example with cosine similarity",
        file: "rag_custom_embedder.rs",
    },
    // ── Thinking ──
    ExampleMeta {
        id: "thinking_openai",
        name: "Reasoning Effort (OpenAI)",
        category: "Thinking",
        description: "Responses API with o4-mini — Low/Medium/High reasoning effort + detailed summaries",
        file: "thinking_openai.rs",
    },
    ExampleMeta {
        id: "thinking_anthropic",
        name: "Extended Thinking (Anthropic)",
        category: "Thinking",
        description: "Claude's internal reasoning with 10K token budget — deep systems design analysis",
        file: "thinking_anthropic.rs",
    },
    ExampleMeta {
        id: "thinking_deepseek",
        name: "Chain-of-Thought (DeepSeek)",
        category: "Thinking",
        description: "Visible chain-of-thought reasoning — watch the model think through math problems",
        file: "thinking_deepseek.rs",
    },
    ExampleMeta {
        id: "thinking_xai",
        name: "Grok Thinking (xAI)",
        category: "Thinking",
        description: "OpenAI-compatible reasoning — Grok-3-mini thinks through Fermi estimation with tools",
        file: "thinking_xai.rs",
    },
    ExampleMeta {
        id: "thinking_gemini",
        name: "Thought Signatures (Gemini)",
        category: "Thinking",
        description: "Native thinking traces + thought_signature on tool calls — multi-turn with preserved context",
        file: "thinking_gemini.rs",
    },
    // ── Advanced ──
    ExampleMeta {
        id: "artifact_agent",
        name: "Artifact Storage",
        category: "Advanced",
        description: "Agent with versioned file storage — save, load, and list artifacts mid-conversation",
        file: "artifact_agent.rs",
    },
    ExampleMeta {
        id: "memory_agent",
        name: "Long-Term Memory",
        category: "Advanced",
        description: "Cross-session memory recall — agent remembers past conversations and preferences",
        file: "memory_agent.rs",
    },
    ExampleMeta {
        id: "guardrails_advanced",
        name: "Advanced Guardrails",
        category: "Advanced",
        description: "PII redaction, content filtering, and GuardrailSet with LLM agent integration",
        file: "guardrails_advanced.rs",
    },
    ExampleMeta {
        id: "auth_rbac",
        name: "RBAC Access Control",
        category: "Advanced",
        description: "Role-based tool permissions — analyst can search but not delete, admin gets full access",
        file: "auth_rbac.rs",
    },
    // ── Security ──
    ExampleMeta {
        id: "auth_identity",
        name: "Typed Identity",
        category: "Security",
        description: "Injection-proof identity system — validated IDs, null-byte rejection, multi-tenant session isolation",
        file: "auth_identity.rs",
    },
    ExampleMeta {
        id: "auth_audit",
        name: "Audit Trail",
        category: "Security",
        description: "Tamper-evident access logging — RBAC permission matrix, AuditSink, AuthMiddleware tool protection",
        file: "auth_audit.rs",
    },
    ExampleMeta {
        id: "auth_sso",
        name: "SSO & JWT",
        category: "Security",
        description: "Enterprise identity — Google/Azure/Okta SSO, JWT validation, OIDC discovery, claims-to-RBAC mapping",
        file: "auth_sso.rs",
    },
    // ── Built-in Tools ──
    ExampleMeta {
        id: "builtin_gemini",
        name: "Google Search (Gemini)",
        category: "Built-in Tools",
        description: "GoogleSearchTool wrapper — server-side search, grounding metadata, and thought signatures across multi-turn tool use",
        file: "builtin_gemini.rs",
    },
    ExampleMeta {
        id: "builtin_anthropic",
        name: "Web Search (Anthropic)",
        category: "Built-in Tools",
        description: "WebSearchTool wrapper for Claude — server-side search with local function tools across multiple turns",
        file: "builtin_anthropic.rs",
    },
    ExampleMeta {
        id: "builtin_openai",
        name: "Web Search (OpenAI)",
        category: "Built-in Tools",
        description: "OpenAIWebSearchTool wrapper — hosted search with local function tools across multiple turns",
        file: "builtin_openai.rs",
    },
    // ── Payments ──
    ExampleMeta {
        id: "payments_checkout",
        name: "Checkout Agent",
        category: "Payments",
        description: "AI-driven checkout lifecycle — create session, select fulfillment, authorize payment, verify status with evidence trail",
        file: "payments_checkout.rs",
    },
    ExampleMeta {
        id: "payments_guardrails",
        name: "Payment Guardrails",
        category: "Payments",
        description: "Amount thresholds, merchant allowlists, policy sets, card/PII redaction, evidence references",
        file: "payments_guardrails.rs",
    },
    ExampleMeta {
        id: "payments_agent",
        name: "Shopping Agent",
        category: "Payments",
        description: "LLM agent with checkout tools — browse, cart, guardrail-enforced payment, masked transaction status",
        file: "payments_agent.rs",
    },
    // ── Action Nodes ──
    // ── Competitive ──
    ExampleMeta {
        id: "competitive_auto_provider",
        name: "Auto-Provider + Encryption",
        category: "Competitive",
        description: "provider_from_env() auto-detects API keys + EncryptedSession with AES-256-GCM at-rest encryption",
        file: "competitive_auto_provider.rs",
    },
    ExampleMeta {
        id: "competitive_graph_resume",
        name: "Durable Graph Resume",
        category: "Competitive",
        description: "MemoryCheckpointer saves graph state — resume from checkpoint after crash, skip completed nodes",
        file: "competitive_graph_resume.rs",
    },
    ExampleMeta {
        id: "competitive_tool_search",
        name: "Tool Search Filter",
        category: "Competitive",
        description: "ToolSearchConfig regex filtering — hide dangerous tools from the LLM while keeping them registered",
        file: "competitive_tool_search.rs",
    },
    // ── Anthropic Features ──
    ExampleMeta {
        id: "anthropic_caching",
        name: "Prompt Caching",
        category: "Anthropic",
        description: "Multi-turn agent with prompt caching — cache creation (25% surcharge) then cache hit (90% discount)",
        file: "anthropic_caching.rs",
    },
    ExampleMeta {
        id: "anthropic_vision",
        name: "Vision Agent",
        category: "Anthropic",
        description: "Image analysis agent — Claude sees images via URL and logs structured observations with tools",
        file: "anthropic_vision.rs",
    },
    ExampleMeta {
        id: "anthropic_structured",
        name: "Structured Extraction",
        category: "Anthropic",
        description: "Typed JSON extraction from unstructured text — tool schema forces structured output",
        file: "anthropic_structured.rs",
    },
    ExampleMeta {
        id: "anthropic_streaming",
        name: "Streaming + Tools",
        category: "Anthropic",
        description: "Real-time streaming with mid-stream tool calls — time-to-first-token metrics",
        file: "anthropic_streaming.rs",
    },
    ExampleMeta {
        id: "anthropic_token_counting",
        name: "Token Counting & Models",
        category: "Anthropic",
        description: "Model discovery, pre-flight token counting, cost estimation, and rate limit info",
        file: "anthropic_token_counting.rs",
    },
    ExampleMeta {
        id: "anthropic_multi_tool",
        name: "Multi-Tool Agent",
        category: "Anthropic",
        description: "Travel assistant with weather, calculator, and unit converter — parallel tool orchestration",
        file: "anthropic_multi_tool.rs",
    },
    ExampleMeta {
        id: "anthropic_thinking_graph",
        name: "Thinking Graph",
        category: "Anthropic",
        description: "Extended thinking (10K budget) in a StateGraph — deep thinker → concise summarizer pipeline",
        file: "anthropic_thinking_graph.rs",
    },
    // ── Action Nodes ──
    ExampleMeta {
        id: "action_set_transform",
        name: "Data Enrichment",
        category: "Action Nodes",
        description: "SET + TRANSFORM action nodes prep data, then an LLM agent writes personalized outreach",
        file: "action_set_transform.rs",
    },
    ExampleMeta {
        id: "action_switch_loop",
        name: "Smart Ticket Router",
        category: "Action Nodes",
        description: "LLM classifier + deterministic SWITCH routing + specialist agents handle support tickets",
        file: "action_switch_loop.rs",
    },
    ExampleMeta {
        id: "action_workflow",
        name: "Content Pipeline",
        category: "Action Nodes",
        description: "SET → Research Agent → TRANSFORM → Writer Agent → SWITCH → Editor Agent — full content pipeline",
        file: "action_workflow.rs",
    },
];

pub fn load_examples() -> Vec<Example> {
    let examples_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");

    REGISTRY
        .iter()
        .filter_map(|meta| {
            let path = examples_dir.join(meta.file);
            let code = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| format!("// Error loading {}: {}", meta.file, e));
            Some(Example {
                id: meta.id.to_string(),
                name: meta.name.to_string(),
                category: meta.category.to_string(),
                description: meta.description.to_string(),
                code,
            })
        })
        .collect()
}
