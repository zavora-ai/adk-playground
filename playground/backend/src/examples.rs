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
        description: "Multi-node graph with state channels (no LLM needed)",
        file: "graph_workflow.rs",
    },
    ExampleMeta {
        id: "graph_conditional",
        name: "Conditional Routing",
        category: "Graph",
        description: "Priority-based conditional edge routing (no LLM needed)",
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
