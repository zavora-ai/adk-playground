//! Recorded Claude answers, so the app runs with no key.
//!
//! A live run records every answer the agent, the vehicle spotter and the clip review get;
//! a replay hands the same answers back in the same places. Answers are keyed by what is
//! stable between machines — the incident a request is about and its turn, or the text it
//! asks — never by image digests, which depend on how ffmpeg decodes the video.

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

use crate::watch::Spot;

/// What the agent says in a `serve` replay when asked something the recording does not hold.
pub const UNRECORDED: &str =
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
    /// Vehicle spots by camera and time.
    spots: BTreeMap<String, BTreeMap<u64, Vec<Spot>>>,
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
    /// An agent request the recording does not hold gets [`UNRECORDED`] instead of an error.
    unrecorded_in_words: AtomicBool,
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
            unrecorded_in_words: AtomicBool::new(false),
        }))
    }

    /// Answers an agent request the recording does not hold with [`UNRECORDED`], so a question
    /// or a decision's closing turn in the browser gets a reply instead of failing. The headless
    /// run keeps the miss an error: it is the check that a replay matches its recording. The
    /// spotter and the clip review always fail loudly.
    pub fn answer_unrecorded_in_words(&self) {
        self.unrecorded_in_words.store(true, Ordering::Relaxed);
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

    pub fn record_spots(&self, camera: &str, at_ms: u64, spots: &[Spot]) {
        let mut recorded = self.recorded.lock().unwrap_or_else(|e| e.into_inner());
        recorded
            .spots
            .entry(camera.to_string())
            .or_default()
            .insert(at_ms, spots.to_vec());
    }

    pub fn spots(&self, camera: &str, at_ms: u64) -> Option<Vec<Spot>> {
        let recorded = self.recorded.lock().unwrap_or_else(|e| e.into_inner());
        recorded.spots.get(camera)?.get(&at_ms).cloned()
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
        // The agent's requests are about one incident; its id names them on every machine.
        let base = match texts.iter().find_map(|t| incident_in(t)) {
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

/// The first incident id in `text`: `c` and two digits, `-`, six digits (`c15-084800`).
pub fn incident_in(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    (0..bytes.len().saturating_sub(9)).find_map(|i| {
        let w = &bytes[i..i + 10];
        let ok = w[0] == b'c'
            && w[1].is_ascii_digit()
            && w[2].is_ascii_digit()
            && w[3] == b'-'
            && w[4..].iter().all(u8::is_ascii_digit)
            && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric());
        ok.then(|| &text[i..i + 10])
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
                        && self.tape.unrecorded_in_words.load(Ordering::Relaxed) =>
                    {
                        tracing::info!(tape.key = %key, "no recorded answer; the replay says so");
                        // A cost of zero, so the kernel's spending cap commits nothing for it.
                        let mut reply =
                            LlmResponse::new(Content::new("model").with_text(UNRECORDED));
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
    use super::*;

    /// A model a replay never reaches.
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

    fn empty_replay(in_words: bool) -> Arc<Tape> {
        Arc::new(Tape {
            mode: Mode::Replay,
            path: PathBuf::new(),
            recorded: Mutex::new(Recorded::default()),
            counts: Mutex::new(HashMap::new()),
            spend: Mutex::new(BTreeMap::new()),
            unrecorded_in_words: AtomicBool::new(in_words),
        })
    }

    async fn ask(tape: &Arc<Tape>, role: &'static str) -> adk_core::Result<Vec<LlmResponse>> {
        let llm = TapedLlm::new(Arc::new(Unreachable), Arc::clone(tape), role);
        let request = LlmRequest::new(
            "m",
            vec![Content::new("user").with_text("Was c01-022799 a near-miss?")],
        );
        let stream = llm.generate_content(request, false).await?;
        stream.collect::<Vec<_>>().await.into_iter().collect()
    }

    #[tokio::test]
    async fn serve_replay_answers_an_unrecorded_agent_request_in_words() {
        let responses = ask(&empty_replay(true), "agent").await.unwrap();
        assert_eq!(
            responses
                .iter()
                .map(|r| r.content.clone())
                .collect::<Vec<_>>(),
            vec![Some(Content::new("model").with_text(UNRECORDED))]
        );
        // Priced at zero, so the kernel's spending cap does not commit a reserved bound.
        assert_eq!(
            responses[0].usage_metadata.as_ref().and_then(|u| u.cost),
            Some(0.0)
        );
    }

    #[tokio::test]
    async fn headless_replay_keeps_an_unrecorded_agent_request_an_error() {
        let error = ask(&empty_replay(false), "agent").await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("no recorded answer for agent:c01-022799:1#1")
        );
    }

    #[tokio::test]
    async fn spotter_and_clip_review_fail_loudly_even_in_serve() {
        let tape = empty_replay(true);
        for role in ["spot", "video"] {
            assert!(ask(&tape, role).await.is_err(), "{role} must fail loudly");
        }
    }

    #[test]
    fn finds_the_incident_id() {
        assert_eq!(
            incident_in("Review near-miss c15-084800 now"),
            Some("c15-084800")
        );
        assert_eq!(
            incident_in("{\"incident\":\"c01-000200\"}"),
            Some("c01-000200")
        );
        assert_eq!(incident_in("abc15-084800"), None);
        assert_eq!(incident_in("nothing here"), None);
    }
}
