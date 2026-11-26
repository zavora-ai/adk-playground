//! The agent's two tools, each confined to one directory.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use adk_core::{AdkError, Result, Tool, ToolContext, ToolEffect};
use async_trait::async_trait;
use serde_json::{Value, json};

/// The most of a file the agent reads.
const READ_LIMIT: usize = 8 * 1024;

/// Reads one file from the inbox directory.
pub struct ReadInboxFile {
    inbox: PathBuf,
}

impl ReadInboxFile {
    pub fn new(inbox: &Path) -> Self {
        Self {
            inbox: inbox.to_path_buf(),
        }
    }
}

#[async_trait]
impl Tool for ReadInboxFile {
    fn name(&self) -> &str {
        "read_inbox_file"
    }

    fn description(&self) -> &str {
        "Reads a text file from the inbox. `path` is the path from the wake notice."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"],
        }))
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn execute(&self, _ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let asked = args["path"].as_str().unwrap_or_default();
        // A wake payload comes from outside the agent, so the path is checked, not trusted:
        // only a file whose resolved location is inside the inbox is read.
        let path = tokio::fs::canonicalize(self.inbox.join(asked))
            .await
            .map_err(|error| AdkError::tool(format!("cannot open {asked}: {error}")))?;
        if !path.starts_with(&self.inbox) {
            return Err(AdkError::tool(format!("{asked} is outside the inbox")));
        }
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|error| AdkError::tool(format!("cannot read {asked}: {error}")))?;
        let text = String::from_utf8_lossy(&bytes[..bytes.len().min(READ_LIMIT)]).into_owned();
        Ok(json!({ "path": path, "text": text, "truncated": bytes.len() > READ_LIMIT }))
    }
}

/// Files one triage note in the notes directory.
pub struct WriteNote {
    notes: PathBuf,
}

impl WriteNote {
    pub fn new(notes: &Path) -> Self {
        Self {
            notes: notes.to_path_buf(),
        }
    }
}

#[async_trait]
impl Tool for WriteNote {
    fn name(&self) -> &str {
        "write_note"
    }

    fn description(&self) -> &str {
        "Files a triage note for one inbox file. `file` is the inbox file's name."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "file": { "type": "string" },
                "summary": { "type": "string" },
                "urgency": { "type": "string", "enum": ["urgent", "routine"] },
                "next_action": { "type": "string" },
            },
            "required": ["file", "summary", "urgency", "next_action"],
        }))
    }

    // Writing the same note twice leaves the same file, so a recovered attempt may repeat it.
    fn effect(&self) -> ToolEffect {
        ToolEffect::Idempotent
    }

    async fn execute(&self, _ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let field = |name: &str| args[name].as_str().unwrap_or_default().trim().to_string();
        let stem = Path::new(&field("file"))
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .filter(|stem| !stem.is_empty() && !stem.starts_with('.'))
            .ok_or_else(|| AdkError::tool("`file` must name the inbox file"))?;
        let note = self.notes.join(format!("{stem}.md"));
        let text = format!(
            "# {}\n\n- urgency: {}\n- summary: {}\n- next action: {}\n",
            field("file"),
            field("urgency"),
            field("summary"),
            field("next_action"),
        );
        tokio::fs::write(&note, text)
            .await
            .map_err(|error| AdkError::tool(format!("cannot write the note: {error}")))?;
        Ok(json!({ "note": note, "status": "filed" }))
    }
}
