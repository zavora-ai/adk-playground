//! Recorded Claude answers, so the app runs with no key.
//!
//! A live run records every answer the agent and the damage assessor get; a replay hands the same
//! answers back in the same places. Answers are keyed by what is stable between machines —
//! the claim a request is about and its turn, or the text it asks — never by image
//! digests.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
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

/// What a replay does when the agent asks something the recording does not hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Miss {
    /// Fails the call: a headless run is the check that the replay is the recorded day.
    Fail,
    /// Answers with [`UNRECORDED`], so a turn the person started in the browser — a
    /// decision's closing turn, a question — ends in words instead of failing silently.
    Say,
}

/// The agent's answer to a request the recording does not hold, under [`Miss::Say`].
pub const UNRECORDED: &str =
    "This is a replay, and this answer was not recorded. Run with --live for a real answer.";

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
    miss: Miss,
    path: PathBuf,
    recorded: Mutex<Recorded>,
    counts: Mutex<HashMap<String, usize>>,
    spend: Mutex<BTreeMap<String, Spend>>,
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
    pub fn open(path: &Path, mode: Mode, miss: Miss) -> anyhow::Result<Arc<Self>> {
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
            miss,
            path: path.to_path_buf(),
            recorded: Mutex::new(recorded),
            counts: Mutex::new(HashMap::new()),
            spend: Mutex::new(BTreeMap::new()),
        }))
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
        // The agent's requests are about one claim; its id names them on every machine.
        let base = match texts.iter().find_map(|t| claim_in(t)) {
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

/// The first claim id in `text`: `CL-` and four digits, standing alone (`CL-0412`).
pub fn claim_in(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    (0..bytes.len().saturating_sub(6)).find_map(|i| {
        let w = &bytes[i..i + 7];
        let ok = &w[..3] == b"CL-"
            && w[3..].iter().all(u8::is_ascii_digit)
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
                let recorded = self.tape.recorded.lock().unwrap_or_else(|e| e.into_inner());
                match recorded.answers.get(&key) {
                    Some(answers) => answers.clone(),
                    // Only the agent answers in words: the assessor's answer is data the
                    // agent acts on, so a miss there still fails loudly.
                    None if self.role == "agent" && self.tape.miss == Miss::Say => {
                        // A reported cost of zero, or the kernel's spend ledger commits the
                        // call's reserved bound against the daily cap.
                        let mut said =
                            LlmResponse::new(Content::new("model").with_text(UNRECORDED));
                        said.usage_metadata = Some(UsageMetadata {
                            cost: Some(0.0),
                            ..UsageMetadata::default()
                        });
                        vec![said]
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

    use super::{Miss, Mode, Tape, TapedLlm, UNRECORDED, claim_in};

    /// The model a replay never calls.
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

    /// What `role` answers, replaying an empty recording under `miss`, to a question about
    /// claim CL-0413.
    async fn replayed(role: &'static str, miss: Miss) -> adk_core::Result<Vec<String>> {
        let path = std::env::temp_dir().join(format!(
            "claims-desk-tape-{}-{role}-{miss:?}.json",
            std::process::id()
        ));
        std::fs::write(&path, r#"{"answers":{}}"#)
            .map_err(|e| adk_core::AdkError::model(e.to_string()))?;
        let tape = Tape::open(&path, Mode::Replay, miss)
            .map_err(|e| adk_core::AdkError::model(e.to_string()))?;
        let _ = std::fs::remove_file(&path);
        let llm = TapedLlm::new(Arc::new(Unreachable), tape, role);
        let request = LlmRequest::new(
            "replay",
            vec![Content::new("user").with_text("Why is CL-0413 major?")],
        );
        let mut answers = Vec::new();
        let mut stream = llm.generate_content(request, false).await?;
        while let Some(response) = stream.next().await {
            for part in response?.content.map(|c| c.parts).unwrap_or_default() {
                if let Part::Text { text } = part {
                    answers.push(text);
                }
            }
        }
        Ok(answers)
    }

    #[tokio::test]
    async fn a_served_replay_says_an_agent_answer_was_not_recorded() {
        assert_eq!(
            replayed("agent", Miss::Say).await.unwrap(),
            vec![UNRECORDED.to_string()]
        );
    }

    #[tokio::test]
    async fn a_headless_replay_and_the_assessor_fail_on_a_miss() {
        assert!(replayed("agent", Miss::Fail).await.is_err());
        assert!(replayed("assessor", Miss::Say).await.is_err());
    }

    #[test]
    fn finds_the_claim_id() {
        assert_eq!(
            claim_in("New claim CL-0412 with two photos"),
            Some("CL-0412")
        );
        assert_eq!(claim_in("{\"claim\":\"CL-0401\"}"), Some("CL-0401"));
        assert_eq!(claim_in("XCL-0412 and CL-04123"), None);
    }
}
