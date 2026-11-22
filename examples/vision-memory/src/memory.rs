//! The visual memory itself: SigLIP 2 embeddings in a persistent vector store, the images as
//! artifacts on disk, RF-DETR labels, and a manifest from each image back to its file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use adk_artifact::FileArtifactService;
use adk_rag::Scope;
use adk_vision::memory::{Erasure, VisualHit, VisualItem, VisualKind, VisualMemory, VisualQuery};
use adk_vision::onnx::{OnnxOptions, RfDetr, SigLip2};
use adk_vision::{
    DecodeLimits, DetectQuery, FetchConfig, InMemoryAcceptanceStore, ManifestSet, MediaScope,
    MediaStore, ModelFetcher, Provenance, SourceImage, VisionCx, VisionRuntime,
};
use chrono::{DateTime, Local, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::store::FileVectorStore;

pub const APP: &str = "vision-memory";
pub const USER: &str = "me";
const COLLECTION: &str = "my-images";
const EMBEDDER: &str = "siglip2-base-patch16-256-quantized";
const IMAGE_TYPES: [&str; 4] = ["png", "jpg", "jpeg", "webp"];
/// The labels RF-DETR adds to an image's caption.
const LABELS: [&str; 14] = [
    "person",
    "cup",
    "bottle",
    "wine glass",
    "cell phone",
    "laptop",
    "keyboard",
    "mouse",
    "book",
    "potted plant",
    "scissors",
    "clock",
    "vase",
    "teddy bear",
];

/// One indexed image: where it came from, so an answer can cite its file.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Indexed {
    pub file: PathBuf,
    pub source: String,
    pub kind: String,
    pub captured_at: DateTime<Utc>,
    pub artifact: String,
}

pub struct Recall {
    memory: VisualMemory,
    media: MediaStore,
    labeler: Arc<VisionRuntime>,
    scope: Scope,
    manifest_path: PathBuf,
    /// Indexed images by digest.
    manifest: Mutex<BTreeMap<String, Indexed>>,
    /// The folders `index_folder` and `recall_like` may read.
    allowed: Vec<PathBuf>,
}

fn fetch(id: &str) -> anyhow::Result<adk_vision::VerifiedModel> {
    let manifest = ManifestSet::builtin()?
        .get(id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("{id} is not in adk-vision's built-in manifest"))?;
    Ok(ModelFetcher::new(
        FetchConfig::from_env(),
        Arc::new(InMemoryAcceptanceStore::new()),
    )
    .fetch_blocking(&manifest)?)
}

fn kind_of(text: &str) -> VisualKind {
    match text {
        "frame" => VisualKind::Frame,
        "page" => VisualKind::Page,
        _ => VisualKind::Image,
    }
}

impl Recall {
    /// Opens the memory under `root`: models fetched and checked, the collection's embedding
    /// space checked against SigLIP 2's.
    pub async fn open(root: &Path, allowed: Vec<PathBuf>) -> anyhow::Result<Self> {
        std::fs::create_dir_all(root)?;
        let (siglip, detr) = tokio::task::spawn_blocking(|| -> anyhow::Result<_> {
            let siglip = SigLip2::new(&fetch(EMBEDDER)?, &OnnxOptions::default())?;
            let detr =
                RfDetr::new(&fetch("rf-detr-n")?, &OnnxOptions::default())?.with_threshold(0.4);
            Ok((siglip, detr))
        })
        .await??;
        let embedder = Arc::new(
            VisionRuntime::builder()
                .embedder(Arc::new(siglip))
                .build()?,
        );
        let labeler = Arc::new(VisionRuntime::builder().detector(Arc::new(detr)).build()?);
        let artifacts = Arc::new(FileArtifactService::new(root.join("images"))?);
        let store = Arc::new(FileVectorStore::open(&root.join("vectors.json")).await?);
        let memory = VisualMemory::builder(embedder, store, COLLECTION)
            .artifacts(artifacts.clone())
            .open()
            .await?;
        let manifest_path = root.join("manifest.json");
        let manifest = match std::fs::read(&manifest_path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(_) => BTreeMap::new(),
        };
        let allowed = allowed
            .iter()
            .filter_map(|dir| dir.canonicalize().ok())
            .collect();
        Ok(Self {
            memory,
            media: MediaStore::new(artifacts),
            labeler,
            scope: Scope::from_parts([USER])?,
            manifest_path,
            manifest: Mutex::new(manifest),
            allowed,
        })
    }

    fn manifest(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Indexed>> {
        self.manifest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn save_manifest(&self) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec_pretty(&*self.manifest())?;
        std::fs::write(&self.manifest_path, bytes)?;
        Ok(())
    }

    /// `path`, if it lies inside a folder the memory may read.
    pub fn permitted(&self, path: &Path) -> anyhow::Result<PathBuf> {
        let path = path
            .canonicalize()
            .map_err(|e| anyhow::anyhow!("{} cannot be read: {e}", path.display()))?;
        if self.allowed.iter().any(|root| path.starts_with(root)) {
            Ok(path)
        } else {
            anyhow::bail!(
                "{} is outside the folders this memory may read: {}",
                path.display(),
                self.allowed
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }

    /// Indexes every image in `folder` as `source`: stored as an artifact, labelled by RF-DETR
    /// and embedded by SigLIP 2, all on this computer. A `.json` beside an image may give its
    /// `caption` and `capturedAt`, as `vision-live-desk` writes them.
    pub async fn index_folder(
        &self,
        folder: &Path,
        source: &str,
        kind: &str,
    ) -> anyhow::Result<Value> {
        let folder = self.permitted(folder)?;
        anyhow::ensure!(
            !source.is_empty()
                && source
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)),
            "a source is a short name of letters, digits, '-', '_' or '.'"
        );
        let mut files: Vec<PathBuf> = std::fs::read_dir(&folder)?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| {
                path.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| IMAGE_TYPES.contains(&e.to_ascii_lowercase().as_str()))
            })
            .collect();
        files.sort();
        let cx = VisionCx::new();
        let scope = MediaScope::new(APP, USER, source);
        let (mut indexed, mut unchanged) = (0, 0);
        let mut items = Vec::new();
        for file in &files {
            let bytes = std::fs::read(file)?;
            let digest = adk_vision::Digest::of(&bytes).to_string();
            if self
                .manifest()
                .get(&digest)
                .is_some_and(|known| known.source == source)
            {
                unchanged += 1;
                continue;
            }
            let sidecar: Value = std::fs::read(file.with_extension("json"))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or(Value::Null);
            let captured_at = sidecar["capturedAt"]
                .as_str()
                .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
                .map(|t| t.to_utc())
                .or_else(|| {
                    file.metadata()
                        .and_then(|m| m.modified())
                        .ok()
                        .map(DateTime::<Utc>::from)
                })
                .unwrap_or_else(Utc::now);
            let (image, pixels) = self
                .media
                .put_image(&scope, bytes, Provenance::Upload)
                .await?;
            let found = self
                .labeler
                .detect(
                    &cx,
                    &pixels,
                    &DetectQuery::Classes(LABELS.iter().map(|l| (*l).to_string()).collect()),
                )
                .await?
                .value;
            let mut labels: Vec<String> = found.iter().filter_map(|r| r.label.clone()).collect();
            labels.sort();
            labels.dedup();
            let name = file
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("image")
                .to_string();
            let note = sidecar["caption"].as_str().unwrap_or("").trim().to_string();
            let caption = match (note.is_empty(), labels.is_empty()) {
                (true, true) => name.clone(),
                (true, false) => format!("{name}; detected on device: {}", labels.join(", ")),
                (false, true) => format!("{name}: {note}"),
                (false, false) => {
                    format!("{name}: {note}; detected on device: {}", labels.join(", "))
                }
            };
            self.manifest().insert(
                digest,
                Indexed {
                    file: file.clone(),
                    source: source.to_string(),
                    kind: kind.to_string(),
                    captured_at,
                    artifact: format!(
                        "artifact://{}/{}/{}/{}#{}",
                        image.artifact.app_name,
                        image.artifact.user_id,
                        image.artifact.session_id,
                        image.artifact.file_name,
                        image.artifact.version
                    ),
                },
            );
            items.push(
                VisualItem::new(image, pixels, caption, source, captured_at)
                    .with_kind(kind_of(kind))
                    .with_labels(labels)
                    .with_run(format!("index-{source}")),
            );
            indexed += 1;
        }
        for batch in items.chunks(8) {
            self.memory.index(&cx, &self.scope, batch).await?;
        }
        self.save_manifest()?;
        Ok(json!({
            "folder": folder.display().to_string(),
            "source": source,
            "kind": kind,
            "indexed": indexed,
            "alreadyIndexed": unchanged,
        }))
    }

    fn query(
        &self,
        top_k: usize,
        source: Option<&str>,
        kind: Option<&str>,
        day: Option<&str>,
    ) -> anyhow::Result<VisualQuery> {
        let mut query = VisualQuery::top(top_k.clamp(1, 20));
        if let Some(source) = source.filter(|s| !s.is_empty()) {
            query = query.source(source);
        }
        if let Some(kind) = kind.filter(|k| !k.is_empty()) {
            query = query.kind(kind_of(kind));
        }
        if let Some(day) = day.filter(|d| !d.is_empty()) {
            query = query.captured_on([NaiveDate::parse_from_str(day, "%Y-%m-%d")?]);
        }
        Ok(query)
    }

    fn cite(&self, hits: Vec<VisualHit>) -> Value {
        let manifest = self.manifest();
        let hits: Vec<Value> = hits
            .into_iter()
            .enumerate()
            .map(|(rank, hit)| {
                let digest = hit.digest.map(|d| d.to_string()).unwrap_or_default();
                let file = manifest.get(&digest).map(|i| i.file.display().to_string());
                json!({
                    "rank": rank + 1,
                    "score": (hit.score * 10_000.0).round() / 10_000.0,
                    "file": file,
                    "source": hit.source,
                    "kind": hit.kind.as_str(),
                    "capturedAt": hit.captured_at.map(|t| t.with_timezone(&Local).format("%a %Y-%m-%d %H:%M %Z").to_string()),
                    "caption": hit.caption,
                    "labels": hit.labels,
                    "artifact": hit.artifact_uri,
                    "digest": digest,
                })
            })
            .collect();
        json!({ "hits": hits })
    }

    /// Searches by text. With `contrast`, each hit also carries its score for the contrasting
    /// text and the margin between the two, so "a red mug" can be told from "a blue mug": a
    /// positive margin means the image looks more like what was asked.
    pub async fn recall_text(
        &self,
        text: &str,
        top_k: usize,
        source: Option<&str>,
        kind: Option<&str>,
        day: Option<&str>,
        contrast: Option<&str>,
    ) -> anyhow::Result<Value> {
        let query = self.query(top_k, source, kind, day)?;
        let cx = VisionCx::new();
        let hits = self
            .memory
            .search_text(&cx, &self.scope, text, &query)
            .await?
            .value;
        let mut answer = self.cite(hits);
        if let Some(contrast) = contrast.filter(|c| !c.trim().is_empty()) {
            let wide = self.query(200, source, kind, day)?;
            let other: BTreeMap<String, f32> = self
                .memory
                .search_text(&cx, &self.scope, contrast, &wide)
                .await?
                .value
                .into_iter()
                .filter_map(|hit| Some((hit.digest?.to_string(), hit.score)))
                .collect();
            for hit in answer["hits"].as_array_mut().into_iter().flatten() {
                let score = hit["score"].as_f64().unwrap_or_default() as f32;
                if let Some(versus) = hit["digest"].as_str().and_then(|d| other.get(d)) {
                    hit["contrastScore"] = json!((versus * 10_000.0).round() / 10_000.0);
                    hit["margin"] = json!(((score - versus) * 10_000.0).round() / 10_000.0);
                }
            }
            answer["contrast"] = json!(contrast);
        }
        Ok(answer)
    }

    /// Searches by an example image: a file in a permitted folder, or an indexed image's digest.
    pub async fn recall_like(
        &self,
        example: &str,
        top_k: usize,
        source: Option<&str>,
        kind: Option<&str>,
    ) -> anyhow::Result<Value> {
        let path = if example.starts_with("sha256:") {
            self.manifest()
                .get(example)
                .map(|i| i.file.clone())
                .ok_or_else(|| anyhow::anyhow!("no indexed image has digest {example}"))?
        } else {
            PathBuf::from(example)
        };
        let path = self.permitted(&path)?;
        let image = SourceImage::decode(&std::fs::read(&path)?, &DecodeLimits::default())?;
        let query = self.query(top_k, source, kind, None)?;
        let hits = self
            .memory
            .search_image(&VisionCx::new(), &self.scope, &image, &query)
            .await?
            .value;
        let mut answer = self.cite(hits);
        answer["example"] = json!(path.display().to_string());
        Ok(answer)
    }

    pub fn sources(&self) -> Value {
        let mut by_source: BTreeMap<String, (usize, String, DateTime<Utc>, DateTime<Utc>)> =
            BTreeMap::new();
        for indexed in self.manifest().values() {
            let entry = by_source.entry(indexed.source.clone()).or_insert((
                0,
                indexed.kind.clone(),
                indexed.captured_at,
                indexed.captured_at,
            ));
            entry.0 += 1;
            entry.2 = entry.2.min(indexed.captured_at);
            entry.3 = entry.3.max(indexed.captured_at);
        }
        let sources: Vec<Value> = by_source
            .into_iter()
            .map(|(source, (images, kind, first, last))| {
                json!({
                    "source": source,
                    "kind": kind,
                    "images": images,
                    "first": first.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string(),
                    "last": last.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string(),
                })
            })
            .collect();
        json!({ "sources": sources, "folders": self.allowed.iter().map(|p| p.display().to_string()).collect::<Vec<_>>() })
    }

    /// Erases everything stored from `source`: its vectors, its stored images, its manifest.
    pub async fn forget(&self, source: &str) -> anyhow::Result<Erasure> {
        let erasure = self
            .memory
            .erase_source(&VisionCx::new(), &self.scope, source)
            .await?;
        self.manifest()
            .retain(|_, indexed| indexed.source != source);
        self.save_manifest()?;
        Ok(erasure)
    }
}
