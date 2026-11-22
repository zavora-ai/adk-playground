//! The agent's tools over the memory. Searching is read-only; erasing a source waits for the
//! person.

use std::path::PathBuf;
use std::sync::Arc;

use adk_core::{Tool, ToolContext, ToolEffect};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::memory::Recall;

fn tool_error(error: anyhow::Error) -> adk_core::AdkError {
    adk_core::AdkError::tool(error.to_string())
}

fn text<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn top_k(args: &Value) -> usize {
    args.get("topK").and_then(Value::as_u64).unwrap_or(6) as usize
}

const FILTERS: &str = r#"{
  "source": { "type": "string", "description": "Only this source, as memory_sources lists it." },
  "kind": { "type": "string", "enum": ["frame", "page", "image"], "description": "frame: camera snapshots; page: scanned documents; image: photos." }
}"#;

fn schema(own: Value, required: &[&str]) -> Value {
    let mut properties: serde_json::Map<String, Value> =
        serde_json::from_str(FILTERS).expect("the filter schema parses");
    if let Value::Object(own) = own {
        properties.extend(own);
    }
    properties.insert(
        "topK".into(),
        json!({ "type": "integer", "minimum": 1, "maximum": 20, "description": "How many hits; 6 by default." }),
    );
    json!({ "type": "object", "properties": properties, "required": required })
}

pub struct Sources(pub Arc<Recall>);

#[async_trait]
impl Tool for Sources {
    fn name(&self) -> &str {
        "memory_sources"
    }
    fn description(&self) -> &str {
        "Lists what the memory holds: each source, its kind, how many images and the time they span, and the folders it may index."
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, _ctx: Arc<dyn ToolContext>, _args: Value) -> adk_core::Result<Value> {
        Ok(self.0.sources())
    }
}

pub struct IndexFolder(pub Arc<Recall>);

#[async_trait]
impl Tool for IndexFolder {
    fn name(&self) -> &str {
        "index_folder"
    }
    fn description(&self) -> &str {
        "Indexes the images in a folder under a source name: each is stored, labelled by RF-DETR and embedded by SigLIP 2 on this computer. Images already indexed are skipped."
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::Idempotent
    }
    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "folder": { "type": "string", "description": "A folder memory_sources lists, or one inside it." },
                "source": { "type": "string", "description": "A short name for where the images come from, such as desk-camera or receipts." },
                "kind": { "type": "string", "enum": ["frame", "page", "image"] },
            },
            "required": ["folder", "source", "kind"],
        }))
    }
    async fn execute(&self, _ctx: Arc<dyn ToolContext>, args: Value) -> adk_core::Result<Value> {
        let folder = PathBuf::from(text(&args, "folder").unwrap_or_default());
        let source = text(&args, "source").unwrap_or_default();
        let kind = text(&args, "kind").unwrap_or("image");
        self.0
            .index_folder(&folder, source, kind)
            .await
            .map_err(tool_error)
    }
}

pub struct RecallText(pub Arc<Recall>);

#[async_trait]
impl Tool for RecallText {
    fn name(&self) -> &str {
        "recall"
    }
    fn description(&self) -> &str {
        "Finds images by a short visual description, such as \"a red coffee mug on a desk\". Returns hits ranked by SigLIP 2 similarity, each with its file, source, capture time, artifact and digest; with `contrast`, also its margin over the contrasting description."
    }
    fn is_read_only(&self) -> bool {
        true
    }
    fn parameters_schema(&self) -> Option<Value> {
        Some(schema(
            json!({
                "text": { "type": "string", "description": "What the image shows, in a few words." },
                "day": { "type": "string", "description": "Only images captured on this day, YYYY-MM-DD." },
                "contrast": { "type": "string", "description": "The closest thing it could be confused with, such as \"a blue coffee mug on a desk\": each hit then carries its margin over it." },
            }),
            &["text"],
        ))
    }
    async fn execute(&self, _ctx: Arc<dyn ToolContext>, args: Value) -> adk_core::Result<Value> {
        self.0
            .recall_text(
                text(&args, "text").unwrap_or_default(),
                top_k(&args),
                text(&args, "source"),
                text(&args, "kind"),
                text(&args, "day"),
                text(&args, "contrast"),
            )
            .await
            .map_err(tool_error)
    }
}

pub struct RecallLike(pub Arc<Recall>);

#[async_trait]
impl Tool for RecallLike {
    fn name(&self) -> &str {
        "recall_like"
    }
    fn description(&self) -> &str {
        "Finds images that look like an example: a file the person names in a folder the memory may read, or an indexed image's digest."
    }
    fn is_read_only(&self) -> bool {
        true
    }
    fn parameters_schema(&self) -> Option<Value> {
        Some(schema(
            json!({ "example": { "type": "string", "description": "A path to the example image, or sha256:… of an indexed image." } }),
            &["example"],
        ))
    }
    async fn execute(&self, _ctx: Arc<dyn ToolContext>, args: Value) -> adk_core::Result<Value> {
        self.0
            .recall_like(
                text(&args, "example").unwrap_or_default(),
                top_k(&args),
                text(&args, "source"),
                text(&args, "kind"),
            )
            .await
            .map_err(tool_error)
    }
}

pub struct Forget(pub Arc<Recall>);

#[async_trait]
impl Tool for Forget {
    fn name(&self) -> &str {
        "forget_source"
    }
    fn description(&self) -> &str {
        "Erases everything the memory holds from one source: its vectors and its stored images. It cannot be undone, and the person is asked first."
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::NonIdempotent
    }
    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": { "source": { "type": "string", "description": "The source, as memory_sources lists it." } },
            "required": ["source"],
        }))
    }
    async fn execute(&self, _ctx: Arc<dyn ToolContext>, args: Value) -> adk_core::Result<Value> {
        let source = text(&args, "source").unwrap_or_default();
        let erasure = self.0.forget(source).await.map_err(tool_error)?;
        Ok(json!({
            "source": erasure.source,
            "chunksDeleted": erasure.chunks,
            "imagesDeleted": erasure.artifacts.len(),
            "imagesKept": erasure.artifacts_kept.len(),
        }))
    }
}
