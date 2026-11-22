//! A vector store that survives a restart: adk-rag's in-memory store, with every change
//! written through to one JSON file.
//!
//! It keeps the example free of a database. For more than a few thousand images, swap in
//! `adk_rag::LanceDBVectorStore` (feature `lancedb`, needs `protoc`) or `PgVectorStore`:
//! `VisualMemory` uses only the `VectorStore` trait, so nothing else changes.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use adk_rag::{
    Chunk, CollectionSpec, Filter, InMemoryVectorStore, Result, Scope, SearchResult, VectorQuery,
    VectorStore,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    collections: BTreeMap<String, SavedCollection>,
}

#[derive(Default, Serialize, Deserialize)]
struct SavedCollection {
    dimensions: usize,
    filterable: BTreeSet<String>,
    /// Chunks by scope key, then by id.
    scopes: BTreeMap<String, BTreeMap<String, Chunk>>,
}

/// adk-rag's `InMemoryVectorStore`, persisted to `path` after every change.
pub struct FileVectorStore {
    inner: InMemoryVectorStore,
    saved: Mutex<Saved>,
    path: PathBuf,
}

fn rag(error: impl std::fmt::Display) -> adk_rag::RagError {
    adk_rag::RagError::VectorStoreError {
        backend: "file".into(),
        message: format!("the store file could not be written or read: {error}"),
    }
}

impl FileVectorStore {
    /// Opens the store at `path`, loading what an earlier run wrote.
    ///
    /// # Errors
    ///
    /// A file that exists but is not a store this example wrote.
    pub async fn open(path: &Path) -> Result<Self> {
        let saved: Saved = match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(rag)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Saved::default(),
            Err(error) => return Err(rag(error)),
        };
        let inner = InMemoryVectorStore::new();
        for (name, collection) in &saved.collections {
            let spec = CollectionSpec::new(collection.dimensions)
                .with_filterable(collection.filterable.iter().cloned());
            inner.create_collection(name, &spec).await?;
            for (scope, chunks) in &collection.scopes {
                let chunks: Vec<Chunk> = chunks.values().cloned().collect();
                inner
                    .upsert(name, &Scope::new(scope.clone())?, &chunks)
                    .await?;
            }
        }
        Ok(Self {
            inner,
            saved: Mutex::new(saved),
            path: path.to_path_buf(),
        })
    }

    fn change(&self, edit: impl FnOnce(&mut Saved)) -> Result<()> {
        let mut saved = self
            .saved
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        edit(&mut saved);
        let bytes = serde_json::to_vec(&*saved).map_err(rag)?;
        let partial = self.path.with_extension("json.partial");
        std::fs::write(&partial, bytes).map_err(rag)?;
        std::fs::rename(&partial, &self.path).map_err(rag)
    }
}

#[async_trait]
impl VectorStore for FileVectorStore {
    async fn create_collection(&self, name: &str, spec: &CollectionSpec) -> Result<()> {
        self.inner.create_collection(name, spec).await?;
        self.change(|saved| {
            let collection = saved.collections.entry(name.to_string()).or_default();
            collection.dimensions = spec.dimensions;
            collection.filterable = spec.filterable.clone();
        })
    }

    async fn delete_collection(&self, name: &str) -> Result<()> {
        self.inner.delete_collection(name).await?;
        self.change(|saved| {
            saved.collections.remove(name);
        })
    }

    async fn upsert(&self, collection: &str, scope: &Scope, chunks: &[Chunk]) -> Result<()> {
        self.inner.upsert(collection, scope, chunks).await?;
        self.change(|saved| {
            let stored = saved
                .collections
                .entry(collection.to_string())
                .or_default()
                .scopes
                .entry(scope.as_str().to_string())
                .or_default();
            for chunk in chunks {
                stored.insert(chunk.id.clone(), chunk.clone());
            }
        })
    }

    async fn delete(&self, collection: &str, scope: &Scope, ids: &[&str]) -> Result<()> {
        self.inner.delete(collection, scope, ids).await?;
        self.change(|saved| {
            if let Some(stored) = saved
                .collections
                .get_mut(collection)
                .and_then(|c| c.scopes.get_mut(scope.as_str()))
            {
                for id in ids {
                    stored.remove(*id);
                }
            }
        })
    }

    async fn search(
        &self,
        collection: &str,
        scope: &Scope,
        query: &VectorQuery<'_>,
    ) -> Result<Vec<SearchResult>> {
        self.inner.search(collection, scope, query).await
    }

    async fn count(&self, collection: &str, scope: &Scope, filter: Option<&Filter>) -> Result<u64> {
        self.inner.count(collection, scope, filter).await
    }

    async fn delete_where(&self, collection: &str, scope: &Scope, filter: &Filter) -> Result<u64> {
        let deleted = self.inner.delete_where(collection, scope, filter).await?;
        self.change(|saved| {
            if let Some(stored) = saved
                .collections
                .get_mut(collection)
                .and_then(|c| c.scopes.get_mut(scope.as_str()))
            {
                stored.retain(|_, chunk| !filter.matches(chunk));
            }
        })?;
        Ok(deleted)
    }

    async fn purge_scope(&self, collection: &str, scope: &Scope) -> Result<u64> {
        let purged = self.inner.purge_scope(collection, scope).await?;
        self.change(|saved| {
            if let Some(c) = saved.collections.get_mut(collection) {
                c.scopes.remove(scope.as_str());
            }
        })?;
        Ok(purged)
    }
}
