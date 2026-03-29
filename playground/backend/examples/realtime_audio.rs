// Realtime Voice — OpenAI Realtime API with streaming audio
//
// Demonstrates:
// - OpenAI Realtime API (gpt-4o-realtime) via WebSocket
// - Text input → streaming voice audio output
// - PCM16 audio chunks streamed to browser for live playback
// - Full WAV saved at end for replay
//
// Requires: OPENAI_API_KEY

use adk_realtime::{
    openai::OpenAIRealtimeModel, RealtimeConfig, RealtimeModel, RealtimeSessionExt, ServerEvent,
};
use base64::Engine;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");
    let api_key = std::env::var("OPENAI_API_KEY")?;

    // Signal the frontend to start the audio player (24kHz PCM16 mono)
    println!("<!--AUDIO_STREAM_START:24000-->");

    println!("🎙️ **Realtime Voice** — OpenAI Realtime API\n");

    let model = OpenAIRealtimeModel::new(&api_key, "gpt-4o-mini-realtime-preview-2024-12-17");

    let config = RealtimeConfig::default()
        .with_instruction(
            "You are a warm, expressive storyteller. The user will give you a topic. \
             Tell a vivid, captivating micro-story (about 30 seconds of speech). \
             Use dramatic pauses, varied pacing, and emotional inflection. \
             Keep it concise but memorable.",
        )
        .with_voice("shimmer")
        .with_modalities(vec!["text".to_string(), "audio".to_string()]);

    println!("📡 Connecting to OpenAI Realtime API...");
    let session = model.connect(config).await?;
    println!("✅ Connected\n");

    let prompt = "A lighthouse keeper who discovers a message in a bottle from the future";
    println!("💬 Prompt: *{}*\n", prompt);
    session.send_text(prompt).await?;
    session.create_response().await?;

    let mut audio_bytes: Vec<u8> = Vec::new();
    let b64 = base64::engine::general_purpose::STANDARD;

    print!("🗣️ ");
    while let Some(event) = session.next_event().await {
        match event? {
            ServerEvent::AudioDelta { delta, .. } => {
                // Stream each audio chunk to the browser for live playback
                let encoded = b64.encode(&delta);
                println!("<!--AUDIO_CHUNK:{}-->", encoded);
                audio_bytes.extend_from_slice(&delta);
            }
            ServerEvent::TranscriptDelta { delta, .. } => {
                print!("{}", delta);
            }
            ServerEvent::ResponseDone { .. } => {
                println!("\n");
                break;
            }
            ServerEvent::Error { error, .. } => {
                println!("\n❌ Error: {}", error.message);
                break;
            }
            _ => {}
        }
    }

    println!("<!--AUDIO_STREAM_END-->");

    if audio_bytes.is_empty() {
        println!("⚠️ No audio received");
        return Ok(());
    }

    let duration_s = audio_bytes.len() as f64 / (24000.0 * 2.0);
    let size_kb = audio_bytes.len() / 1024;
    println!("✅ Audio: {duration_s:.1}s · 24000Hz · {size_kb}KB\n");

    // Also save full WAV for replay after streaming ends
    let wav = pcm16_to_wav(&audio_bytes, 24000, 1);
    let audio_dir = std::path::PathBuf::from("audio-output");
    std::fs::create_dir_all(&audio_dir)?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let filename = format!("realtime-{ts}.wav");
    std::fs::write(audio_dir.join(&filename), &wav)?;

    println!("<!--AUDIO_URL:/api/audio/{filename}-->");

    Ok(())
}

fn pcm16_to_wav(pcm: &[u8], sample_rate: u32, channels: u16) -> Vec<u8> {
    let bits_per_sample: u16 = 16;
    let byte_rate = sample_rate * channels as u32 * (bits_per_sample / 8) as u32;
    let block_align = channels * (bits_per_sample / 8);
    let data_size = pcm.len() as u32;
    let file_size = 36 + data_size;
    let mut wav = Vec::with_capacity(44 + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&file_size.to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&bits_per_sample.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    wav.extend_from_slice(pcm);
    wav
}
