//! A model with no network: fixed rules in place of a language model, so the example runs
//! without an API key. It reads the same request a real model receives — the kernel's wake
//! notice included — and answers with the same tool calls.

use adk_core::{
    Content, FinishReason, Llm, LlmRequest, LlmResponse, LlmResponseStream, Part, Result,
};
use async_trait::async_trait;
use serde_json::{Value, json};

/// The delimiters the kernel puts around a wake's payload, one line of JSON between them.
const PAYLOAD_OPEN: &str = "<<<wake-payload\n";
const PAYLOAD_CLOSE: &str = "\nwake-payload>>>";

pub struct RuleModel;

#[async_trait]
impl Llm for RuleModel {
    fn name(&self) -> &str {
        "offline-rules"
    }

    async fn generate_content(
        &self,
        request: LlmRequest,
        _stream: bool,
    ) -> Result<LlmResponseStream> {
        let parts = request.contents.iter().flat_map(|content| &content.parts);
        let said: Vec<&str> = parts.clone().filter_map(Part::text).collect();
        let answer = |tool: &str| {
            parts.clone().find_map(|part| match part {
                Part::FunctionResponse {
                    function_response, ..
                } if function_response.name == tool => Some(function_response.response.clone()),
                _ => None,
            })
        };
        let call = |name: &str, args: Value| Part::FunctionCall {
            name: name.into(),
            args,
            id: Some(format!("offline-{name}")),
            thought_signature: None,
        };

        let part = match (
            wake_payload(&said),
            answer("read_inbox_file"),
            answer("write_note"),
        ) {
            (None, _, _) => text("No wake notice reached this turn, so there is nothing to do."),
            (Some(event), None, _) => call("read_inbox_file", json!({ "path": event["path"] })),
            (Some(event), Some(read), None) => {
                let body = read["text"].as_str().unwrap_or_default();
                let lower = body.to_lowercase();
                let urgent = ["urgent", "failed", "outage", "down"]
                    .iter()
                    .any(|word| lower.contains(word));
                let file = event["path"].as_str().unwrap_or_default();
                call(
                    "write_note",
                    json!({
                        "file": file.rsplit(['/', '\\']).next().unwrap_or(file),
                        "summary": body.lines().next().unwrap_or_default(),
                        "urgency": if urgent { "urgent" } else { "routine" },
                        "next_action": if urgent { "page the on-call owner" } else { "read at the next review" },
                    }),
                )
            }
            (Some(_), Some(_), Some(filed)) => text(&format!("Filed {}.", filed["note"])),
        };

        let mut response = LlmResponse::new(Content {
            role: "model".into(),
            parts: vec![part],
        });
        response.finish_reason = Some(FinishReason::Stop);
        response.turn_complete = true;
        Ok(Box::pin(futures::stream::iter([Ok(response)])))
    }
}

fn text(text: &str) -> Part {
    Part::Text { text: text.into() }
}

/// The payload of the wake notice in the turn's text, when the kernel added one.
fn wake_payload(said: &[&str]) -> Option<Value> {
    let notice = said.iter().find(|text| text.contains("[kernel wake \""))?;
    let start = notice.find(PAYLOAD_OPEN)? + PAYLOAD_OPEN.len();
    let end = start + notice[start..].find(PAYLOAD_CLOSE)?;
    serde_json::from_str(&notice[start..end]).ok()
}
