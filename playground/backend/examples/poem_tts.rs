// Poem → Speech — LLM composes a poem, Gemini TTS reads it aloud
//
// Demonstrates:
// - LLM agent generating creative content
// - Gemini TTS text-to-speech synthesis
// - Audio output served to the browser for playback
//
// Requires: GOOGLE_API_KEY

use adk_audio::{AudioFormat, CloudTtsConfig, GeminiTts, TtsProvider, TtsRequest, encode};
use adk_rust::prelude::*;
use adk_rust::session::{SessionService, CreateRequest};
use adk_rust::futures::StreamExt;
use adk_core::{UserId, SessionId};
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")?;

    // ── Step 1: Compose a poem with Gemini ──
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    let agent = Arc::new(
        LlmAgentBuilder::new("poet")
            .instruction(
                "You are a world-class poet in the tradition of Mary Oliver, Pablo Neruda, \
                 and Rumi. Write an original poem of 12–20 lines. Choose a vivid theme — \
                 the ocean at dawn, a forgotten garden, starlight on snow, rain on a tin roof, \
                 the passage of time, or a childhood memory. Use rich imagery, metaphor, and \
                 sensory detail. Vary line length for rhythm. End with a resonant closing image. \
                 Output ONLY the poem — no title, no attribution, no commentary."
            )
            .model(model)
            .build()?
    );

    let sessions = Arc::new(InMemorySessionService::new());
    sessions.create(CreateRequest {
        app_name: "playground".into(),
        user_id: "user".into(),
        session_id: Some("s1".into()),
        state: HashMap::new(),
    }).await?;

    let runner = Runner::new(RunnerConfig {
        app_name: "playground".into(),
        agent,
        session_service: sessions,
        artifact_service: None,
        memory_service: None,
        plugin_manager: None,
        run_config: None,
        compaction_config: None,
        context_cache_config: None,
        cache_capable: None,
        request_context: None,
        cancellation_token: None,
    })?;

    println!("🎭 Composing a poem...\n");

    let message = Content::new("user").with_text("Write me a beautiful poem.");
    let mut stream = runner.run(UserId::new("user")?, SessionId::new("s1")?, message).await?;
    let mut poem = String::new();
    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                if let Some(text) = part.text() {
                    poem.push_str(text);
                }
            }
        }
    }
    let poem = poem.trim().to_string();

    // Format as a Markdown blockquote with line breaks preserved
    // (two trailing spaces = <br> in Markdown)
    let md_poem: String = poem
        .lines()
        .map(|line| {
            if line.trim().is_empty() {
                ">".to_string()
            } else {
                format!("> *{}*  ", line)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    println!("{md_poem}");
    println!();

    // ── Step 2: Read the poem aloud with Gemini TTS ──
    println!("---");
    println!();
    println!("🔊 Synthesizing speech (Gemini TTS, voice: Kore)...");
    println!();

    let tts = GeminiTts::new(CloudTtsConfig::new(api_key));
    let request = TtsRequest {
        text: poem.clone(),
        voice: "Kore".into(),
        ..Default::default()
    };
    let frame = tts.synthesize(&request).await?;
    let wav = encode(&frame, AudioFormat::Wav)?;

    let duration_s = frame.duration_ms as f64 / 1000.0;
    let size_kb = wav.len() / 1024;
    println!("✅ Audio: {duration_s:.1}s · {}Hz · {size_kb}KB", frame.sample_rate);
    println!();

    // Write WAV for the playground server to serve
    let audio_dir = std::path::PathBuf::from("audio-output");
    std::fs::create_dir_all(&audio_dir)?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?.as_secs();
    let filename = format!("poem-{ts}.wav");
    std::fs::write(audio_dir.join(&filename), &wav)?;

    println!("<!--AUDIO_URL:/api/audio/{filename}-->");

    Ok(())
}
