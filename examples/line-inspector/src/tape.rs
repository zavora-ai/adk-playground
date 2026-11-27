//! Recorded Claude answers, so the app runs with no key.
//!
//! A live run records every answer the agent and the two vision models get; a replay hands the same
//! answers back in the same places. Answers are keyed by what is stable between machines —
//! the part a request is about and its turn, or the text it asks — never by image
//! digests.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use adk_core::{
    Content, DataDestination, DataLocality, Llm, LlmRequest, LlmResponse, LlmResponseStream, Part,
    SchemaAdapter,
};
use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The agent's reply in `serve` to a question the recording has no answer for.
pub const NOT_RECORDED: &str =
    "This is a replay, and this answer was not recorded. Run with --live for a real answer.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Live,
    Replay,
}

#[derive(Default, Serialize, Deserialize)]
struct Recorded {
    /// Answers by request key, each the stream of responses one call returned.
    answers: BTreeMap<String, Vec<LlmResponse>>,
    /// What the live run spent, by role.
    #[serde(default)]
    spend: BTreeMap<String, Spend>,
}

/// The recording a live run writes and a replay reads.
pub struct Tape {
    pub mode: Mode,
    path: PathBuf,
    recorded: Mutex<Recorded>,
    counts: Mutex<HashMap<String, usize>>,
    spend: Mutex<BTreeMap<String, Spend>>,
    /// An agent call the recording lacks gets [`NOT_RECORDED`] instead of an error.
    explains_misses: AtomicBool,
}

/// What one role's model calls used, priced from adk-model's catalog.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Spend {
    pub model: String,
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub usd: f64,
}

impl Tape {
    pub fn open(path: &Path, mode: Mode) -> anyhow::Result<Arc<Self>> {
        let recorded = match mode {
            Mode::Live => Recorded::default(),
            Mode::Replay => {
                let text = std::fs::read_to_string(path).map_err(|e| {
                    anyhow::anyhow!(
                        "{} could not be read ({e}): set ANTHROPIC_API_KEY to run live",
                        path.display()
                    )
                })?;
                serde_json::from_str(&text)?
            }
        };
        Ok(Arc::new(Self {
            mode,
            path: path.to_path_buf(),
            recorded: Mutex::new(recorded),
            counts: Mutex::new(HashMap::new()),
            spend: Mutex::new(BTreeMap::new()),
            explains_misses: AtomicBool::new(false),
        }))
    }

    /// From now on an agent call the recording lacks is answered with [`NOT_RECORDED`], so a
    /// question or a decision's closing turn in `serve` never fails silently. A headless run
    /// keeps the miss an error: it is the replay's determinism check.
    pub fn explain_misses(&self) {
        self.explains_misses.store(true, Ordering::Relaxed);
    }

    /// Writes what a live run recorded.
    pub fn save(&self) -> anyhow::Result<()> {
        if self.mode == Mode::Live {
            let spend = self.spend();
            let mut recorded = self.recorded.lock().unwrap_or_else(|e| e.into_inner());
            recorded.spend = spend;
            std::fs::write(&self.path, serde_json::to_vec(&*recorded)?)?;
        }
        Ok(())
    }

    /// Each role's calls, tokens and cost: this run's when live, the recorded live run's
    /// when replaying, since a replay spends nothing.
    pub fn spend(&self) -> BTreeMap<String, Spend> {
        match self.mode {
            Mode::Live => self.spend.lock().unwrap_or_else(|e| e.into_inner()).clone(),
            Mode::Replay => self
                .recorded
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .spend
                .clone(),
        }
    }

    fn add_spend(&self, role: &str, model: &str, responses: &[LlmResponse]) {
        let mut spend = self.spend.lock().unwrap_or_else(|e| e.into_inner());
        let entry = spend.entry(role.to_string()).or_default();
        entry.model = model.to_string();
        entry.calls += 1;
        let price = adk_model::pricing::PricingCatalog::standard()
            .lookup(Some("anthropic"), model)
            .cloned();
        for usage in responses.iter().filter_map(|r| r.usage_metadata.as_ref()) {
            entry.input_tokens += u64::try_from(usage.prompt_token_count).unwrap_or(0);
            entry.output_tokens += u64::try_from(usage.candidates_token_count).unwrap_or(0);
            if let Some(price) = &price {
                entry.usd += price.cost_usd_at(usage, chrono::Utc::now());
            }
        }
    }

    /// The key a request is recorded under, with its occurrence counted.
    fn key(&self, role: &str, request: &LlmRequest) -> String {
        let texts: Vec<&str> = request
            .contents
            .iter()
            .flat_map(|c| c.parts.iter())
            .filter_map(|p| match p {
                Part::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        // The agent's requests are about one part; its id names them on every machine.
        let base = match texts.iter().find_map(|t| part_in(t)) {
            Some(id) if role == "agent" => format!("{role}:{id}:{}", request.contents.len()),
            _ => {
                let digest = Sha256::digest(texts.join("\n").as_bytes());
                format!("{role}:{}", &hex::encode(digest)[..16])
            }
        };
        let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        let n = counts.entry(base.clone()).or_default();
        *n += 1;
        format!("{base}#{n}")
    }
}

/// The first part id in `text`: `t`, three digits, `-p` and a digit (`t017-p2`).
pub fn part_in(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    (0..bytes.len().saturating_sub(6)).find_map(|i| {
        let w = &bytes[i..i + 7];
        let ok = w[0] == b't'
            && w[1..4].iter().all(u8::is_ascii_digit)
            && &w[4..6] == b"-p"
            && w[6].is_ascii_digit()
            && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric())
            && bytes.get(i + 7).is_none_or(|b| !b.is_ascii_alphanumeric());
        ok.then(|| &text[i..i + 7])
    })
}

/// A model that records its answers live and replays them without a key.
pub struct TapedLlm {
    inner: Arc<dyn Llm>,
    tape: Arc<Tape>,
    role: &'static str,
}

impl TapedLlm {
    pub fn new(inner: Arc<dyn Llm>, tape: Arc<Tape>, role: &'static str) -> Arc<Self> {
        Arc::new(Self { inner, tape, role })
    }
}

#[async_trait]
impl Llm for TapedLlm {
    fn name(&self) -> &str {
        self.inner.name()
    }

    async fn generate_content(
        &self,
        req: LlmRequest,
        stream: bool,
    ) -> adk_core::Result<LlmResponseStream> {
        let key = self.tape.key(self.role, &req);
        let responses = match self.tape.mode {
            Mode::Replay => {
                let recorded = self
                    .tape
                    .recorded
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .answers
                    .get(&key)
                    .cloned();
                match recorded {
                    Some(responses) => responses,
                    None if self.role == "agent"
                        && self.tape.explains_misses.load(Ordering::Relaxed) =>
                    {
                        tracing::info!(tape.key = %key, "no recorded answer; replying that this is a replay");
                        vec![LlmResponse::new(
                            Content::new("model").with_text(NOT_RECORDED),
                        )]
                    }
                    None => {
                        return Err(adk_core::AdkError::model(format!(
                            "no recorded answer for {key}: this machine saw something the \
                             recording did not; set ANTHROPIC_API_KEY to run live"
                        )));
                    }
                }
            }
            Mode::Live => {
                let mut live = self.inner.generate_content(req, stream).await?;
                let mut responses = Vec::new();
                while let Some(response) = live.next().await {
                    responses.push(response?);
                }
                self.tape
                    .recorded
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .answers
                    .insert(key, responses.clone());
                responses
            }
        };
        self.tape
            .add_spend(self.role, self.inner.name(), &responses);
        Ok(Box::pin(futures::stream::iter(
            responses.into_iter().map(Ok),
        )))
    }

    fn schema_adapter(&self) -> &dyn SchemaAdapter {
        self.inner.schema_adapter()
    }

    fn max_cost_micro_usd(&self, request: &LlmRequest) -> Option<u64> {
        self.inner.max_cost_micro_usd(request)
    }

    fn context_window(&self) -> Option<u64> {
        self.inner.context_window()
    }

    fn data_locality(&self) -> DataLocality {
        self.inner.data_locality()
    }

    fn data_destinations(&self) -> Vec<DataDestination> {
        self.inner.data_destinations()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use adk_core::{Content, Llm, LlmRequest, LlmResponseStream};
    use async_trait::async_trait;
    use futures::StreamExt;

    use super::{Mode, NOT_RECORDED, Tape, TapedLlm, part_in};

    struct Unreachable;

    #[async_trait]
    impl Llm for Unreachable {
        fn name(&self) -> &str {
            "unreachable"
        }

        async fn generate_content(
            &self,
            _req: LlmRequest,
            _stream: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            Err(adk_core::AdkError::model("a replay never calls the model"))
        }
    }

    fn empty_tape(name: &str) -> Arc<Tape> {
        let path =
            std::env::temp_dir().join(format!("line-inspector-{name}-{}.json", std::process::id()));
        std::fs::write(&path, r#"{"answers":{}}"#).unwrap();
        let tape = Tape::open(&path, Mode::Replay).unwrap();
        std::fs::remove_file(&path).unwrap();
        tape
    }

    fn question() -> LlmRequest {
        LlmRequest::new(
            "claude-sonnet-5-5",
            vec![Content::new("user").with_text("Why reject t002-p3?")],
        )
    }

    #[tokio::test]
    async fn a_miss_is_an_error_until_the_page_asks_for_replies() {
        let tape = empty_tape("strict");
        let agent = TapedLlm::new(Arc::new(Unreachable), Arc::clone(&tape), "agent");
        assert!(agent.generate_content(question(), false).await.is_err());
    }

    #[tokio::test]
    async fn the_page_gets_a_reply_for_an_agent_miss_only() {
        let tape = empty_tape("lenient");
        tape.explain_misses();
        let agent = TapedLlm::new(Arc::new(Unreachable), Arc::clone(&tape), "agent");
        let responses: Vec<_> = agent
            .generate_content(question(), false)
            .await
            .unwrap()
            .collect()
            .await;
        let content = responses[0].as_ref().unwrap().content.clone().unwrap();
        assert_eq!(content, Content::new("model").with_text(NOT_RECORDED));
        let reasoner = TapedLlm::new(Arc::new(Unreachable), tape, "reasoner");
        assert!(reasoner.generate_content(question(), false).await.is_err());
    }

    #[test]
    fn finds_the_part_id() {
        assert_eq!(
            part_in("Borderline part t017-p2 on tray t017"),
            Some("t017-p2")
        );
        assert_eq!(part_in("{\"id\":\"t080-p4\"}"), Some("t080-p4"));
        assert_eq!(part_in("at017-p2 and t017-p23"), None);
    }
}
