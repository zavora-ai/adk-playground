//! Direct WebSocket clients for the two **dedicated translation** endpoints.
//!
//! These are not the standard realtime/voice-agent APIs (so they don't go
//! through ADK's `IntegratedRealtimeRunner`) — they're purpose-built interpreter
//! endpoints:
//!
//! - **OpenAI `gpt-realtime-translate`** — `wss://api.openai.com/v1/realtime/translations`.
//!   Continuous interpreter: you send `session.input_audio_buffer.append`, set the
//!   target language via `session.update` (`session.audio.output.language`), and
//!   receive `session.output_audio.delta` / `session.{output,input}_transcript.delta`.
//!   There is **no** `response.create`. 24 kHz PCM16 in and out.
//! - **Gemini `gemini-3.5-live-translate-preview`** — the Live API endpoint with a
//!   `generationConfig.translationConfig` (`targetLanguageCode`). 16 kHz PCM16 in,
//!   24 kHz out; transcripts via `inputAudioTranscription`/`outputAudioTranscription`.
//!
//! Each client takes a stream of base64 PCM16 mic frames and emits [`XlatEvent`]s
//! (translated audio + source/target transcripts) back to the bridge.

use anyhow::{Context, Result};
use base64::Engine;
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tracing::{info, warn};

/// Gemini Live (AI Studio) bidirectional endpoint.
const GEMINI_LIVE_URL: &str = "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";

/// The realtime translation provider for a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    OpenAI,
    Gemini,
}

impl Provider {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "gemini" | "google" => Provider::Gemini,
            _ => Provider::OpenAI,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Provider::OpenAI => "openai",
            Provider::Gemini => "gemini",
        }
    }

    /// (input_sample_rate, output_sample_rate) the browser must use.
    pub fn audio_rates(self) -> (u32, u32) {
        match self {
            // gpt-realtime-translate is 24 kHz both ways.
            Provider::OpenAI => (24_000, 24_000),
            // Gemini Live consumes 16 kHz PCM16 and emits 24 kHz PCM16.
            Provider::Gemini => (16_000, 24_000),
        }
    }

    /// Default model id (overridable via env).
    pub fn default_model(self) -> &'static str {
        match self {
            Provider::OpenAI => "gpt-realtime-translate",
            Provider::Gemini => "gemini-3.5-live-translate-preview",
        }
    }
}

/// Events streamed from the provider back toward the browser.
#[derive(Debug, Clone)]
pub enum XlatEvent {
    /// base64 PCM16 translated audio at the provider's output rate.
    Audio(String),
    /// Source-language (what the speaker said) transcript delta.
    Source(String),
    /// Target-language (translated) transcript delta.
    Target(String),
    /// Status / informational message.
    Info(String),
    /// A fatal error; the session is ending.
    Error(String),
}

/// Resolve the API key for a provider from the environment.
pub fn api_key(provider: Provider) -> Option<String> {
    match provider {
        Provider::OpenAI => std::env::var("OPENAI_API_KEY").ok(),
        Provider::Gemini => {
            std::env::var("GEMINI_API_KEY").or_else(|_| std::env::var("GOOGLE_API_KEY")).ok()
        }
    }
    .filter(|k| !k.trim().is_empty())
}

/// Model id for a provider (env override, else the preview default).
pub fn model_for(provider: Provider) -> String {
    let env = match provider {
        Provider::OpenAI => "OPENAI_TRANSLATE_MODEL",
        Provider::Gemini => "GEMINI_TRANSLATE_MODEL",
    };
    std::env::var(env).unwrap_or_else(|_| provider.default_model().to_string())
}

/// Drive a translation session for one connection: pump `audio_rx` (base64 PCM16
/// mic frames) up to the provider and forward provider events to `evt`.
pub async fn run(
    provider: Provider,
    api_key: String,
    model: String,
    target_lang: String,
    audio_rx: mpsc::Receiver<String>,
    evt: mpsc::Sender<XlatEvent>,
) -> Result<()> {
    match provider {
        Provider::OpenAI => run_openai(api_key, model, target_lang, audio_rx, evt).await,
        Provider::Gemini => run_gemini(api_key, model, target_lang, audio_rx, evt).await,
    }
}

// ─── OpenAI gpt-realtime-translate ───────────────────────────────────────────

async fn run_openai(
    api_key: String,
    model: String,
    target_lang: String,
    mut audio_rx: mpsc::Receiver<String>,
    evt: mpsc::Sender<XlatEvent>,
) -> Result<()> {
    let url = format!("wss://api.openai.com/v1/realtime/translations?model={model}");
    let mut request = url.as_str().into_client_request().context("invalid OpenAI URL")?;
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {api_key}")).context("bad API key header")?,
    );
    let (ws, _) = connect_async(request).await.context("OpenAI translate connect failed")?;
    let (mut write, mut read) = ws.split();
    info!(model = %model, target = %target_lang, "openai translate: connected");

    // Set the target output language for the interpreter.
    let update = json!({
        "type": "session.update",
        "session": { "audio": { "output": { "language": target_lang } } }
    });
    write.send(Message::Text(update.to_string().into())).await?;
    let _ = evt.send(XlatEvent::Info(format!("OpenAI · translating into {target_lang}"))).await;

    // Pump mic audio up as continuous interpreter input (no response.create).
    let pump = tokio::spawn(async move {
        while let Some(b64) = audio_rx.recv().await {
            let msg = json!({ "type": "session.input_audio_buffer.append", "audio": b64 });
            if write.send(Message::Text(msg.to_string().into())).await.is_err() {
                break;
            }
        }
        let _ =
            write.send(Message::Text(json!({ "type": "session.close" }).to_string().into())).await;
    });

    while let Some(msg) = read.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                let _ = evt.send(XlatEvent::Error(e.to_string())).await;
                break;
            }
        };
        let text = match msg {
            Message::Text(t) => t.to_string(),
            Message::Binary(b) => String::from_utf8_lossy(&b).into_owned(),
            Message::Close(frame) => {
                if let Some(f) = &frame {
                    let reason = f.reason.to_string();
                    if !reason.is_empty() {
                        warn!(code = ?f.code, %reason, "provider closed connection");
                        let _ = evt
                            .send(XlatEvent::Error(format!("closed [{:?}]: {reason}", f.code)))
                            .await;
                    }
                }
                break;
            }
            _ => continue,
        };
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let ty = v.get("type").and_then(Value::as_str).unwrap_or_default();
        let delta = v.get("delta").and_then(Value::as_str).map(str::to_string);
        // Match by suffix/substring so minor naming drift in this preview API
        // (e.g. input_transcript vs input_audio_transcript) still routes.
        if ty.ends_with("output_audio.delta") {
            if let Some(d) = delta {
                let _ = evt.send(XlatEvent::Audio(d)).await;
            }
        } else if ty.contains("output") && ty.contains("transcript") {
            if let Some(d) = delta {
                let _ = evt.send(XlatEvent::Target(d)).await;
            }
        } else if ty.contains("input") && ty.contains("transcript") {
            if let Some(d) = delta {
                let _ = evt.send(XlatEvent::Source(d)).await;
            }
        } else if ty == "session.closed" {
            break;
        } else if ty == "error" {
            let m = v
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            let _ = evt.send(XlatEvent::Error(m.to_string())).await;
        }
    }
    pump.abort();
    info!("openai translate: session ended");
    Ok(())
}

// ─── Gemini 3.5 Live Translate ───────────────────────────────────────────────

async fn run_gemini(
    api_key: String,
    model: String,
    target_lang: String,
    mut audio_rx: mpsc::Receiver<String>,
    evt: mpsc::Sender<XlatEvent>,
) -> Result<()> {
    let url = format!("{GEMINI_LIVE_URL}?key={api_key}");
    let (ws, _) = connect_async(url).await.context("Gemini Live connect failed")?;
    let (mut write, mut read) = ws.split();
    info!(model = %model, target = %target_lang, "gemini translate: connected");

    // Configure the live-translate session: audio out, transcripts both ways,
    // and the target language.
    let setup = json!({
        "setup": {
            "model": format!("models/{model}"),
            "generationConfig": {
                "responseModalities": ["AUDIO"],
                // translationConfig is a generationConfig field (not a top-level
                // setup field — the server rejects it there).
                "translationConfig": { "targetLanguageCode": target_lang, "echoTargetLanguage": true }
            },
            // Transcription configs are top-level setup fields (standard Live schema).
            "inputAudioTranscription": {},
            "outputAudioTranscription": {}
        }
    });
    write.send(Message::Text(setup.to_string().into())).await?;
    let _ = evt.send(XlatEvent::Info(format!("Gemini · translating into {target_lang}"))).await;

    // Pump mic audio up as 16 kHz PCM16 realtime input.
    let pump = tokio::spawn(async move {
        while let Some(b64) = audio_rx.recv().await {
            let msg = json!({
                "realtimeInput": { "audio": { "mimeType": "audio/pcm;rate=16000", "data": b64 } }
            });
            if write.send(Message::Text(msg.to_string().into())).await.is_err() {
                break;
            }
        }
    });

    while let Some(msg) = read.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                let _ = evt.send(XlatEvent::Error(e.to_string())).await;
                break;
            }
        };
        let text = match msg {
            Message::Text(t) => t.to_string(),
            // Gemini Live frequently delivers server messages as binary JSON.
            Message::Binary(b) => String::from_utf8_lossy(&b).into_owned(),
            Message::Close(frame) => {
                if let Some(f) = &frame {
                    let reason = f.reason.to_string();
                    if !reason.is_empty() {
                        warn!(code = ?f.code, %reason, "provider closed connection");
                        let _ = evt
                            .send(XlatEvent::Error(format!("closed [{:?}]: {reason}", f.code)))
                            .await;
                    }
                }
                break;
            }
            _ => continue,
        };
        handle_gemini_message(&text, &evt).await;
    }
    pump.abort();
    info!("gemini translate: session ended");
    Ok(())
}

async fn handle_gemini_message(text: &str, evt: &mpsc::Sender<XlatEvent>) {
    let v: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return,
    };

    if v.get("setupComplete").is_some() {
        let _ = evt.send(XlatEvent::Info("Gemini · session ready".into())).await;
        return;
    }

    if let Some(content) = v.get("serverContent") {
        // Translated audio chunks (inline PCM16 @ 24 kHz).
        if let Some(parts) =
            content.get("modelTurn").and_then(|m| m.get("parts")).and_then(Value::as_array)
        {
            for part in parts {
                if let Some(data) =
                    part.get("inlineData").and_then(|i| i.get("data")).and_then(Value::as_str)
                {
                    let _ = evt.send(XlatEvent::Audio(data.to_string())).await;
                }
            }
        }
        // Translated (target) transcript.
        if let Some(t) =
            content.get("outputTranscription").and_then(|o| o.get("text")).and_then(Value::as_str)
        {
            let _ = evt.send(XlatEvent::Target(t.to_string())).await;
        }
        // Source (spoken) transcript.
        if let Some(t) =
            content.get("inputTranscription").and_then(|o| o.get("text")).and_then(Value::as_str)
        {
            let _ = evt.send(XlatEvent::Source(t.to_string())).await;
        }
    }
}

/// Headless probe. With no file, it just connects/configures and confirms the
/// endpoint, auth, and model id are valid. With a raw PCM16 file (matching the
/// provider's input rate — 24 kHz OpenAI, 16 kHz Gemini), it streams that audio
/// in near-real time and reports the source + translated transcripts and how
/// much translated audio came back — a true end-to-end translation test.
///
/// `cargo run -- probe [openai|gemini] [path/to/input.pcm]`
pub async fn probe(provider: Provider, target: String, pcm_file: Option<String>) -> Result<()> {
    let key =
        api_key(provider).with_context(|| format!("missing API key for {}", provider.name()))?;
    let model = model_for(provider);
    let (audio_tx, audio_rx) = mpsc::channel::<String>(256);
    let (evt_tx, mut evt_rx) = mpsc::channel::<XlatEvent>(256);

    info!(provider = provider.name(), model = %model, target = %target, "probe: connecting");
    let session = tokio::spawn(run(provider, key, model, target, audio_rx, evt_tx));

    // Stream the PCM file (if any) as paced ~40 ms frames; otherwise just drop
    // the sender so a connectivity-only probe doesn't hang.
    let streaming = pcm_file.is_some();
    if let Some(path) = pcm_file {
        let bytes = std::fs::read(&path).with_context(|| format!("reading {path}"))?;
        let (in_rate, _) = provider.audio_rates();
        let frame = (in_rate as usize * 2 / 25).max(2); // 40 ms of PCM16
        info!(file = %path, bytes = bytes.len(), "probe: streaming PCM16 input (~realtime)");
        tokio::spawn(async move {
            for chunk in bytes.chunks(frame) {
                let b64 = base64::engine::general_purpose::STANDARD.encode(chunk);
                if audio_tx.send(b64).await.is_err() {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            }
            // Let trailing audio settle, then drop the sender to flush/close.
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        });
    } else {
        drop(audio_tx);
    }

    let mut source = String::new();
    let mut translation = String::new();
    let mut audio_bytes = 0usize;
    let mut ready = false;
    // Connectivity-only probes finish fast; streaming probes wait for output.
    let budget = if streaming { 40 } else { 4 };
    let deadline = tokio::time::sleep(std::time::Duration::from_secs(budget));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            ev = evt_rx.recv() => match ev {
                Some(XlatEvent::Error(e)) => anyhow::bail!("provider error: {e}"),
                Some(XlatEvent::Info(m)) => { info!("probe: {m}"); ready = true; }
                Some(XlatEvent::Source(s)) => source.push_str(&s),
                Some(XlatEvent::Target(s)) => translation.push_str(&s),
                Some(XlatEvent::Audio(b64)) => {
                    audio_bytes +=
                        base64::engine::general_purpose::STANDARD.decode(&b64).map(|d| d.len()).unwrap_or(0);
                }
                None => break, // session ended
            },
            _ = &mut deadline => break,
        }
    }
    session.abort();

    if streaming {
        info!(
            provider = provider.name(),
            heard = %source.trim(),
            translation = %translation.trim(),
            audio_bytes,
            "probe: complete"
        );
        anyhow::ensure!(
            !translation.trim().is_empty() || audio_bytes > 0,
            "no translation produced"
        );
    } else {
        anyhow::ensure!(ready, "no session-ready signal received");
        info!(provider = provider.name(), "probe: ok — endpoint/auth/model valid");
    }
    Ok(())
}
