//! Recorded Claude answers, so the app runs with no key.
//!
//! A live run records every answer the agent and the crop reader get; a replay hands the same
//! answers back in the same places. Answers are keyed by what is stable between machines —
//! the document a request is about and its turn, or the text it asks — never by image
//! digests.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use adk_core::{
    Content, DataDestination, DataLocality, Llm, LlmRequest, LlmResponse, LlmResponseStream, Part,
    SchemaAdapter, UsageMetadata,
};
use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
    /// A replay in the browser answers an unrecorded agent turn in words instead of failing it.
    agent_misses_answered: AtomicBool,
}

/// The agent's reply, in a replay in the browser, to a turn the recording does not hold: a
/// decision's closing turn or a question asked on the page.
pub const NOT_RECORDED: &str =
    "This is a replay, and this answer was not recorded. Run with --live for a real answer.";

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
            agent_misses_answered: AtomicBool::new(false),
        }))
    }

    /// From now on, a replayed agent request the recording does not hold is answered with
    /// [`NOT_RECORDED`] instead of an error. `serve` sets it, so a decision's closing turn
    /// never fails silently; `run` leaves it off, since there a miss is the determinism check.
    pub fn answer_agent_misses(&self) {
        self.agent_misses_answered.store(true, Ordering::Relaxed);
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
        // The agent's requests are about one document; its id names them on every machine.
        let base = match texts.iter().find_map(|t| doc_in(t)) {
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

/// The first document id in `text`: `d` and two digits, standing alone (`d07`).
pub fn doc_in(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    (0..bytes.len().saturating_sub(2)).find_map(|i| {
        let ok = bytes[i] == b'd'
            && bytes[i + 1].is_ascii_digit()
            && bytes[i + 2].is_ascii_digit()
            && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric())
            && bytes.get(i + 3).is_none_or(|b| !b.is_ascii_alphanumeric());
        ok.then(|| &text[i..i + 3])
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
                        && self.tape.agent_misses_answered.load(Ordering::Relaxed) =>
                    {
                        tracing::info!(tape.key = %key, "unrecorded agent turn answered as a replay");
                        // A zero cost on the reply keeps the kernel's daily cap from
                        // charging a replay for an answer nobody paid for.
                        let mut reply =
                            LlmResponse::new(Content::new("model").with_text(NOT_RECORDED));
                        reply.usage_metadata = Some(UsageMetadata {
                            cost: Some(0.0),
                            ..UsageMetadata::default()
                        });
                        vec![reply]
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

    use adk_core::{Content, Llm, LlmRequest, LlmResponseStream, Part};
    use async_trait::async_trait;
    use futures::StreamExt;

    use super::{Mode, NOT_RECORDED, Tape, TapedLlm, doc_in};

    /// The live model, which a replay never calls.
    struct Unreachable;

    #[async_trait]
    impl Llm for Unreachable {
        fn name(&self) -> &str {
            "unreachable"
        }

        async fn generate_content(
            &self,
            _: LlmRequest,
            _: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            unreachable!("a replay never calls the live model")
        }
    }

    async fn reply(tape: &Arc<Tape>, role: &'static str) -> adk_core::Result<Option<String>> {
        let model = TapedLlm::new(Arc::new(Unreachable), Arc::clone(tape), role);
        let request = LlmRequest::new(
            "unreachable",
            vec![Content::new("user").with_text("What does d07 still need?")],
        );
        let mut stream = model.generate_content(request, false).await?;
        let response = stream.next().await.transpose()?;
        Ok(response
            .and_then(|r| r.content)
            .and_then(|c| c.parts.into_iter().next())
            .and_then(|p| match p {
                Part::Text { text } => Some(text),
                _ => None,
            }))
    }

    #[tokio::test]
    async fn an_unrecorded_agent_turn_fails_in_a_run_and_answers_in_the_browser() {
        let path = std::env::temp_dir().join(format!("ap-desk-tape-{}.json", std::process::id()));
        std::fs::write(&path, r#"{"answers":{}}"#).unwrap();
        let tape = Tape::open(&path, Mode::Replay).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(reply(&tape, "agent").await.is_err());
        tape.answer_agent_misses();
        assert_eq!(
            reply(&tape, "agent").await.unwrap().as_deref(),
            Some(NOT_RECORDED)
        );
        assert!(reply(&tape, "crop").await.is_err());
    }

    #[test]
    fn finds_the_document_id() {
        assert_eq!(doc_in("New invoice d07 in the inbox"), Some("d07"));
        assert_eq!(doc_in("the artifact d12.jpg"), Some("d12"));
        assert_eq!(doc_in("sha256:9ad07f and d3"), None);
        assert_eq!(doc_in("d123"), None);
    }
}
