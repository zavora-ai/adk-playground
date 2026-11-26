# ADK-Rust Teaching Examples

Standalone crates that each teach one ADK-Rust feature. They moved here from the
`examples/` directory of [adk-rust](https://github.com/zavora-ai/adk-rust) and keep
their original directory names, so a link to `examples/<name>` in the ADK-Rust
documentation resolves to the same name in this directory.

These crates are separate from the hosted playground: the single-file examples the
playground runs live in [`playground/backend/examples/`](../playground/backend/examples/).

## Running an example

Each directory is its own Cargo workspace with a committed `Cargo.lock`.

```bash
cd examples/yaml_agent
cargo run
```

Most examples read API keys from a `.env` file in the example directory; each
README lists the variables it needs.

**Requirements:** Rust 1.95.0 or later, the toolchain ADK-Rust 3.0 requires.

## ADK-Rust dependency

Every `adk-*` dependency is a git dependency on the adk-rust repository, pinned to
one commit:

```toml
adk-core = { git = "https://github.com/zavora-ai/adk-rust", rev = "ca01dd8ec861f0d6b321a7c56857adea9cda25d3" }
```

ADK-Rust 3.0.0 is not on crates.io yet and `adk-harness` is unpublished, so a
crates.io version requirement cannot build these crates.

To move every example to another commit, replace the revision in each manifest and
regenerate the lockfiles:

```bash
OLD=ca01dd8ec861f0d6b321a7c56857adea9cda25d3
NEW=<commit>
sed -i "s/$OLD/$NEW/" examples/*/Cargo.toml
for dir in examples/*/; do (cd "$dir" && cargo update --workspace); done
```

`vision-live-desk` and `vision-memory` pin a later commit,
`b526a163c4856f92375df09ee89040ececd42189`, the first with adk-vision's camera wakes and visual
memory; the `sed` above leaves them as they are. `warehouse-watch` pins
`98515241133d24f7e41f8fd272649035416a142f`, the first with the `vision_video` tool, and so does
`ap-desk`, `line-inspector` and `claims-desk`.

`bi_analyst` and `blender_console` depend on `run_console_driver` by relative path
(`../run_console_driver`); keep the three directories together.

## Index

| Directory | Example |
|-----------|---------|
| [`a2a-research-agent`](a2a-research-agent/) | A2A v1.0.0 Research Agent |
| [`a2a-writing-agent`](a2a-writing-agent/) | A2A v1.0.0 Writing Agent |
| [`a2a_interceptors`](a2a_interceptors/) | A2A Interceptors Example |
| [`a2a_quickstart`](a2a_quickstart/) | A2A Quickstart Example |
| [`acp_client_host`](acp_client_host/) | ACP client and host example |
| [`acp_kiro`](acp_kiro/) | ACP coding-agent client examples |
| [`acp_server`](acp_server/) | ACP server example |
| [`action_nodes`](action_nodes/) | Action Nodes Example |
| [`advanced_agents`](advanced_agents/) | Advanced agents in the ADK Runtime |
| [`agent_orchestrator`](agent_orchestrator/) | Agent Orchestrator |
| [`agent_registry`](agent_registry/) | Agent Registry Example |
| [`ambient_cron_agent`](ambient_cron_agent/) | Ambient Cron Agent Example |
| [`ap-desk`](ap-desk/) | AP Desk — supplier invoices read, matched and posted, exceptions put to the clerk |
| [`awp_agent`](awp_agent/) | AWP Agent Example |
| [`background_runs`](background_runs/) | Background Runs Example |
| [`bi_analyst`](bi_analyst/) | BI Analyst — an agent that works a dashboard the way a person does |
| [`bigquery_toolset`](bigquery_toolset/) | BigQuery Toolset Example |
| [`blender_console`](blender_console/) | Blender Console — an agent a person can watch, and interrupt |
| [`blender_studio`](blender_studio/) | Blender Studio — an agent that builds a 3D scene through two MCP servers |
| [`claims-desk`](claims-desk/) | Claims Desk — motor claims assessed from their photos, estimates drafted for the adjuster |
| [`codeact_agent`](codeact_agent/) | CodeAct Agent example |
| [`codeact_monty_agent`](codeact_monty_agent/) | CodeAct × Monty agent example |
| [`coding_agent`](coding_agent/) | Coding Agent example |
| [`coding_goal`](coding_goal/) | Autonomous goal loop — `/goal` with durable checkpointing |
| [`coding_graph`](coding_graph/) | Coding workflow as a graph — "ultra-review" style |
| [`context_compaction`](context_compaction/) | Context Compaction Example |
| [`cron_scheduling`](cron_scheduling/) | Cron Scheduling Example |
| [`customer_service`](customer_service/) | Customer Service Agent — multimodal realtime support |
| [`deepseek_v4`](deepseek_v4/) | DeepSeek V4 Example |
| [`deferred_nodes`](deferred_nodes/) | Deferred Nodes Example |
| [`delta_checkpoints`](delta_checkpoints/) | Delta Checkpoints Example |
| [`desktop_agent`](desktop_agent/) | Desktop Agent Example |
| [`durable_company`](durable_company/) | Durable Company |
| [`enterprise_custom_tool`](enterprise_custom_tool/) | Enterprise Custom Tool Example |
| [`enterprise_hello`](enterprise_hello/) | Enterprise Hello World |
| [`eval_showcase`](eval_showcase/) | Eval Showcase |
| [`example_store`](example_store/) | Example Store — dynamic few-shot retrieval |
| [`functional_workflow`](functional_workflow/) | Functional Workflow Example |
| [`gemini_audio`](gemini_audio/) | Gemini Audio — Agentic TTS & STT |
| [`gemini_interactions_agent`](gemini_interactions_agent/) | Gemini Interactions API — Runtime Agent Example |
| [`gemini_managed_agents`](gemini_managed_agents/) | Gemini Managed Agents — Practical Examples |
| [`graph_goto_routing`](graph_goto_routing/) | Routing chosen by the model |
| [`graph_resilient_research`](graph_resilient_research/) | A research desk the model calls as a tool |
| [`graph_subgraph_claims`](graph_subgraph_claims/) | Claim pipeline built from nested graphs |
| [`harness_pattern`](harness_pattern/) | Harness Pattern — Trait-Based Agent Execution |
| [`ideate_with_meta`](ideate_with_meta/) | Ideate with Meta |
| [`intra_compaction`](intra_compaction/) | Intra-Compaction Example |
| [`kernel_wake_agent`](kernel_wake_agent/) | Kernel Wake Agent — an agent an event starts |
| [`knowledge_graph_agent`](knowledge_graph_agent/) | Knowledge-Graph Memory for a Text Agent |
| [`line-inspector`](line-inspector/) | Line Inspector — every candle on the line inspected, borderline parts given a second look |
| [`live_translation`](live_translation/) | Live Translation — real-time speech-to-speech in the browser |
| [`managed_agents_custom_tools`](managed_agents_custom_tools/) | Managed Agents: Custom Tool Flow |
| [`managed_agents_files`](managed_agents_files/) | Managed Agents: File Upload and Processing |
| [`managed_agents_hello`](managed_agents_hello/) | Managed Agents: Hello World |
| [`managed_agents_memory`](managed_agents_memory/) | Managed Agents: Persistent Memory Across Sessions |
| [`managed_agents_multiagent`](managed_agents_multiagent/) | Managed Agents: Multiagent Coordinator |
| [`mcp_elicitation`](mcp_elicitation/) | MCP Elicitation Example |
| [`mcp_manager`](mcp_manager/) | Dynamic MCP server manager |
| [`mcp_protocol_revisions`](mcp_protocol_revisions/) | MCP protocol revisions |
| [`mcp_resources`](mcp_resources/) | MCP Resources, Prompts & Notifications Example |
| [`mcp_sampling`](mcp_sampling/) | MCP Sampling Compatibility Example |
| [`monty_python_code_tool`](monty_python_code_tool/) | MontyPythonCodeTool Example |
| [`multi_perspective_analysis`](multi_perspective_analysis/) | Multi-Perspective Analysis Example |
| [`multimodal_function_response`](multimodal_function_response/) | Multimodal Function Responses |
| [`muse_agentic`](muse_agentic/) | Muse agentic — Meta Muse Spark agent with tool calling |
| [`node_caching`](node_caching/) | Node Caching Example |
| [`node_timeouts`](node_timeouts/) | Node Timeouts Example |
| [`openai_background`](openai_background/) | OpenAI Background Mode Example |
| [`openai_builtin_tools`](openai_builtin_tools/) | OpenAI Built-In Tools Example |
| [`openai_conversations`](openai_conversations/) | OpenAI Conversations API Example |
| [`openai_deep_research`](openai_deep_research/) | OpenAI Deep Research Example |
| [`openai_open_responses`](openai_open_responses/) | OpenAI Open Responses Example |
| [`openai_ws_minimal`](openai_ws_minimal/) | OpenAI WebSocket Minimal |
| [`opencode`](opencode/) | OpenCode Go and Zen |
| [`openrouter`](openrouter/) | OpenRouter Example |
| [`output_schema_agent`](output_schema_agent/) | Output Schema Agent Example |
| [`parallel_shared_state`](parallel_shared_state/) | Parallel Shared State Example |
| [`payments`](payments/) | Payments Example Index |
| [`plugin_system`](plugin_system/) | Enhanced Plugin System Demo |
| [`project_scoped_memory`](project_scoped_memory/) | Project-Scoped Memory Example |
| [`prompt_optimizer`](prompt_optimizer/) | Prompt Optimizer Example |
| [`realtime_tools`](realtime_tools/) | Realtime Tools — Function Calling in Voice Conversations |
| [`realtime_voice`](realtime_voice/) | Realtime Voice — Mindfulness with Mia |
| [`retry_reflect`](retry_reflect/) | Retry & Reflect Demo |
| [`run_console_driver`](run_console_driver/) | Run console driver — shared library for `bi_analyst` and `blender_console` |
| [`runtime_ui_showcase`](runtime_ui_showcase/) | Embedded runtime UI showcase |
| [`sandbox_agent`](sandbox_agent/) | Sandbox Agent Example |
| [`sandbox_workspace_agent`](sandbox_workspace_agent/) | Sandbox Workspace Agent Example |
| [`schema_normalization`](schema_normalization/) | Schema Normalization Demo |
| [`secret_provider`](secret_provider/) | Secret Provider Example |
| [`server_builder`](server_builder/) | ServerBuilder Example |
| [`skills_multilingual`](skills_multilingual/) | Multilingual Skill Matching |
| [`slack_toolset`](slack_toolset/) | Slack Toolset Example |
| [`spanner_toolset`](spanner_toolset/) | Spanner Toolset Example |
| [`streaming_bash`](streaming_bash/) | Streaming Bash — `BashTool` progress streamed through `LlmAgent` |
| [`team_architectures`](team_architectures/) | Portable team architectures with OpenAI |
| [`telemetry_sqlite_export`](telemetry_sqlite_export/) | Telemetry SQLite Export Example |
| [`time_travel`](time_travel/) | Time Travel Example |
| [`tool_concurrency`](tool_concurrency/) | Tool Concurrency Example |
| [`user_personas`](user_personas/) | User Personas Example |
| [`vertex_rag`](vertex_rag/) | Vertex AI RAG Engine — agentic corpus retrieval |
| [`vertex_sandbox`](vertex_sandbox/) | Vertex AI Agent Engine Sandbox Example |
| [`video_avatar`](video_avatar/) | Video Avatar Example |
| [`vision-live-desk`](vision-live-desk/) | Desk Watch — an agent that watches your desk through a camera |
| [`vision-memory`](vision-memory/) | Recall — an agent with visual memory |
| [`warehouse-watch`](warehouse-watch/) | Warehouse Watch — near-misses on the warehouse floor, put to the shift supervisor |
| [`yaml_agent`](yaml_agent/) | YAML Agent Definition Example |
