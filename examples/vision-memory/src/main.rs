//! Recall: an agent with visual memory.
//!
//! | Command | What runs |
//! |---------|-----------|
//! | `demo-corpus` | Writes a generated week of desk snapshots, scanned pages and two query images |
//! | `index` | Indexes a folder directly, without the agent |
//! | `ask` | One or more questions to the agent, in one conversation |
//! | `chat` | A conversation on the terminal |
//!
//! Images are embedded by SigLIP 2 and labelled by RF-DETR on this computer, and stay there:
//! the agent's model sees captions, scores, times and citations, never pixels.

mod corpus;
mod memory;
mod store;
mod tools;

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;

use adk_agent::LlmAgentBuilder;
use adk_core::{
    Content, Part, RunConfig, SessionId, ToolConfirmationDecision, ToolConfirmationHandler,
    ToolConfirmationRequest, UserId,
};
use adk_runner::Runner;
use adk_session::{CreateRequest, InMemorySessionService, SessionService};
use clap::{Args, Parser, Subcommand, ValueEnum};
use futures::StreamExt;

use crate::memory::{APP, Recall, USER};

const SESSION: &str = "recall";

const INSTRUCTION: &str = r#"You are Recall, the person's visual memory: snapshots their desk camera kept, their photos and their scanned documents, indexed on this computer with SigLIP 2. Searching runs on this computer; you see captions, scores, times and citations, never the images.

How to answer:
- Search with recall using a short visual description of what the image would show ("a red coffee mug on a desk", "an invoice with a totals table"), not the person's question word for word. Use recall_like when the person gives an example image.
- Scores are SigLIP 2 similarities: small, sometimes negative, and only comparable within one search. Treat the hits clearly above the rest as matches; when no hit stands out, say the match is uncertain.
- To tell an object from a similar one (a red mug from a blue one, a receipt from a delivery note), pass the closest alternative as `contrast`: hits with a positive margin look more like what was asked.
- For "when did I last see X": search with a contrast, keep the hits with a positive margin, and answer with the latest capture time among them.
- Cite every image you rely on: file name, source, capture time, and its artifact URI. Never state something no hit supports.
- Captions say only what was detected on the device and what the person or the desk camera noted; the score is what tells you an image looks like the description.
- forget_source erases a source for good; call it only when the person asks to forget something, and it asks them before it runs.
Keep answers short."#;

#[derive(Parser)]
#[command(name = "vision-memory", about = "An agent with visual memory")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Writes a generated corpus: desk snapshots, scanned pages and two query images.
    DemoCorpus {
        #[arg(long, default_value = "demo")]
        out: PathBuf,
    },
    /// Indexes a folder directly, without the agent.
    Index {
        #[command(flatten)]
        common: Common,
        folder: PathBuf,
        #[arg(long)]
        source: String,
        /// frame (camera snapshots), page (scanned documents) or image (photos).
        #[arg(long, default_value = "image")]
        kind: String,
    },
    /// Searches by text or by an example image directly, without the agent.
    Search {
        #[command(flatten)]
        common: Common,
        /// A short visual description, or with `--like`, a path to an example image.
        query: String,
        /// Searches by the example image at `query`.
        #[arg(long)]
        like: bool,
        #[arg(long, default_value_t = 8)]
        top_k: usize,
    },
    /// Asks the agent one or more questions, in one conversation.
    Ask {
        #[command(flatten)]
        common: Common,
        #[arg(required = true)]
        questions: Vec<String>,
    },
    /// Talks with the agent on the terminal.
    Chat {
        #[command(flatten)]
        common: Common,
    },
}

#[derive(Args, Clone)]
struct Common {
    /// Where the memory keeps its vectors, images and manifest.
    #[arg(long, default_value = "memory-store")]
    store: PathBuf,
    /// Folders the memory may read; `demo` and the desk's snapshots by default.
    #[arg(long = "allow")]
    allow: Vec<PathBuf>,
    /// The agent's model.
    #[arg(long, default_value = "claude-sonnet-5-5")]
    model: String,
    /// What happens when the agent asks to forget a source.
    #[arg(long, value_enum, default_value_t = Confirm::Ask)]
    confirm: Confirm,
}

impl Common {
    fn allowed(&self) -> Vec<PathBuf> {
        if self.allow.is_empty() {
            vec![
                PathBuf::from("demo"),
                PathBuf::from("../vision-live-desk/desk/snapshots"),
            ]
        } else {
            self.allow.clone()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Confirm {
    /// Asks on the terminal; declines when stdin is not a terminal.
    Ask,
    /// Approves.
    Yes,
    /// Declines.
    No,
}

/// Decides a held call on the terminal.
#[derive(Debug)]
struct Terminal(Confirm);

#[async_trait::async_trait]
impl ToolConfirmationHandler for Terminal {
    async fn decide(
        &self,
        request: &ToolConfirmationRequest,
    ) -> adk_core::Result<ToolConfirmationDecision> {
        println!(
            "┌ {} wants to run {} {}",
            APP, request.tool_name, request.args
        );
        let approve = match self.0 {
            Confirm::Yes => true,
            Confirm::No => false,
            Confirm::Ask if std::io::stdin().is_terminal() => {
                print!("└ allow it? [y/N] ");
                let _ = std::io::stdout().flush();
                tokio::task::spawn_blocking(|| {
                    let mut line = String::new();
                    let _ = std::io::stdin().read_line(&mut line);
                    line.trim().eq_ignore_ascii_case("y")
                })
                .await
                .unwrap_or(false)
            }
            Confirm::Ask => false,
        };
        println!("└ {}", if approve { "allowed" } else { "declined" });
        Ok(if approve {
            ToolConfirmationDecision::Approve
        } else {
            ToolConfirmationDecision::Deny
        })
    }
}

fn short(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(limit).collect::<String>())
    }
}

async fn agent(common: &Common) -> anyhow::Result<Runner> {
    let key = std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("ANTHROPIC_API_KEY is not set: copy .env.example to .env")
        })?;
    let model = Arc::new(adk_model::anthropic::AnthropicClient::new(
        adk_model::anthropic::AnthropicConfig::new(key, &common.model),
    )?);
    println!("opening the memory (SigLIP 2 and RF-DETR, fetched and checked on first run)…");
    let recall = Arc::new(Recall::open(&common.store, common.allowed()).await?);
    let agent = LlmAgentBuilder::new("recall")
        .description("The person's visual memory")
        .model(model)
        .instruction(format!(
            "{INSTRUCTION}\n\nToday is {}.",
            chrono::Local::now().format("%A %Y-%m-%d, %H:%M %Z")
        ))
        .tool(Arc::new(tools::Sources(recall.clone())))
        .tool(Arc::new(tools::IndexFolder(recall.clone())))
        .tool(Arc::new(tools::RecallText(recall.clone())))
        .tool(Arc::new(tools::RecallLike(recall.clone())))
        .tool(Arc::new(tools::Forget(recall)))
        .require_tool_confirmation("forget_source")
        .build()?;
    let sessions: Arc<dyn SessionService> = Arc::new(InMemorySessionService::new());
    sessions
        .create(CreateRequest {
            app_name: APP.into(),
            user_id: USER.into(),
            session_id: Some(SESSION.into()),
            state: Default::default(),
        })
        .await?;
    Ok(Runner::builder()
        .app_name(APP)
        .agent(Arc::new(agent))
        .session_service(sessions)
        .run_config(
            RunConfig::builder()
                .tool_confirmation_handler(Arc::new(Terminal(common.confirm)))
                .build(),
        )
        .build()?)
}

async fn turn(runner: &Runner, question: &str) -> anyhow::Result<()> {
    println!();
    println!("[you] {question}");
    let mut events = runner
        .run(
            UserId::new(USER)?,
            SessionId::new(SESSION)?,
            Content::new("user").with_text(question),
        )
        .await?;
    while let Some(event) = events.next().await {
        let event = event?;
        let Some(content) = event.llm_response.content else {
            continue;
        };
        for part in content.parts {
            match part {
                Part::FunctionCall { name, args, .. } => println!("[call] {name} {args}"),
                Part::FunctionResponse {
                    function_response, ..
                } => println!(
                    "[answer] {} {}",
                    function_response.name,
                    short(&function_response.response.to_string(), 1200)
                ),
                Part::Text { text } if !text.trim().is_empty() && !event.llm_response.partial => {
                    println!("[recall] {}", text.trim());
                }
                _ => {}
            }
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    adk_core::ensure_crypto_provider();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();
    match Cli::parse().command {
        Command::DemoCorpus { out } => println!("{}", corpus::generate(&out)?),
        Command::Index {
            common,
            folder,
            source,
            kind,
        } => {
            println!(
                "opening the memory (SigLIP 2 and RF-DETR, fetched and checked on first run)…"
            );
            let recall = Recall::open(&common.store, common.allowed()).await?;
            let started = std::time::Instant::now();
            let report = recall.index_folder(&folder, &source, &kind).await?;
            println!("{report} in {:.1?}", started.elapsed());
        }
        Command::Search {
            common,
            query,
            like,
            top_k,
        } => {
            let recall = Recall::open(&common.store, common.allowed()).await?;
            let found = if like {
                recall.recall_like(&query, top_k, None, None).await?
            } else {
                recall
                    .recall_text(&query, top_k, None, None, None, None)
                    .await?
            };
            for hit in found["hits"].as_array().into_iter().flatten() {
                println!(
                    "{:>2}  {:+.4}  {:<11} {:<26} {}",
                    hit["rank"],
                    hit["score"].as_f64().unwrap_or_default(),
                    hit["source"].as_str().unwrap_or_default(),
                    hit["capturedAt"].as_str().unwrap_or_default(),
                    hit["caption"].as_str().unwrap_or_default(),
                );
            }
        }
        Command::Ask { common, questions } => {
            let runner = agent(&common).await?;
            for question in questions {
                turn(&runner, &question).await?;
            }
        }
        Command::Chat { common } => {
            let runner = agent(&common).await?;
            println!("ask about your images; an empty line ends the conversation");
            loop {
                print!("> ");
                std::io::stdout().flush()?;
                let line = tokio::task::spawn_blocking(|| {
                    let mut line = String::new();
                    std::io::stdin().read_line(&mut line).map(|_| line)
                })
                .await??;
                if line.trim().is_empty() {
                    break;
                }
                turn(&runner, line.trim()).await?;
            }
        }
    }
    Ok(())
}
