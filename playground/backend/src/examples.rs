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
        description: "Multi-agent system with coordinator routing to specialists",
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
