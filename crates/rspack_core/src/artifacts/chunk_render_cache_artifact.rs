use futures::Future;
use rspack_collections::Identifier;
use rspack_error::{Diagnostic, Result};
use rspack_sources::BoxSource;

use crate::{
  ArtifactExt, AssetHashRecord, Chunk, Compilation, MemoryGCStorage, SourceType,
  incremental::{Incremental, IncrementalPasses},
};

#[derive(Debug, Default)]
pub struct ChunkRenderCacheArtifact {
  storage: Option<MemoryGCStorage<BoxSource>>,
  real_content_hash_storage: Option<MemoryGCStorage<AssetHashRecord>>,
}

impl ArtifactExt for ChunkRenderCacheArtifact {
  const PASS: IncrementalPasses = IncrementalPasses::CHUNK_ASSET;

  fn recover(_incremental: &Incremental, new: &mut Self, old: &mut Self) {
    *new = std::mem::take(old);
    new.start_next_generation();
  }
}

impl ChunkRenderCacheArtifact {
  pub fn new(storage: MemoryGCStorage<BoxSource>) -> Self {
    let real_content_hash_storage = MemoryGCStorage::new(storage.max_generations());
    Self {
      storage: Some(storage),
      real_content_hash_storage: Some(real_content_hash_storage),
    }
  }
  pub fn start_next_generation(&self) {
    if let Some(storage) = &self.storage {
      storage.start_next_generation();
    }
    if let Some(storage) = &self.real_content_hash_storage {
      storage.start_next_generation();
    }
  }
  pub async fn use_cache<G, F>(
    &self,
    compilation: &Compilation,
    chunk: &Chunk,
    source_type: &SourceType,
    generator: G,
  ) -> Result<(BoxSource, Vec<Diagnostic>)>
  where
    G: FnOnce() -> F,
    F: Future<Output = Result<(BoxSource, Vec<Diagnostic>)>>,
  {
    let Some(storage) = &self.storage else {
      panic!("ChunkRenderCacheArtifact storage is not set");
    };
    let Some(content_hash) =
      chunk.content_hash_by_source_type(&compilation.chunk_hashes_artifact, source_type)
    else {
      return generator().await;
    };
    let cache_key = Identifier::from(content_hash.encoded());
    if let Some(value) = storage.get(&cache_key) {
      Ok((value, Vec::new()))
    } else {
      let res = generator().await?;
      storage.set(cache_key, res.0.clone());
      Ok(res)
    }
  }

  pub async fn use_cache_with_real_content_hashes<G, F>(
    &self,
    compilation: &Compilation,
    chunk: &Chunk,
    source_type: &SourceType,
    generator: G,
  ) -> Result<(BoxSource, AssetHashRecord, Vec<Diagnostic>)>
  where
    G: FnOnce() -> F,
    F: Future<Output = Result<(BoxSource, AssetHashRecord, Vec<Diagnostic>)>>,
  {
    let Some(storage) = &self.storage else {
      panic!("ChunkRenderCacheArtifact storage is not set");
    };
    let Some(real_content_hash_storage) = &self.real_content_hash_storage else {
      panic!("ChunkRenderCacheArtifact real content hash storage is not set");
    };
    let Some(content_hash) =
      chunk.content_hash_by_source_type(&compilation.chunk_hashes_artifact, source_type)
    else {
      return generator().await;
    };
    let cache_key = Identifier::from(content_hash.encoded());
    if let Some(value) = storage.get(&cache_key) {
      let real_content_hashes = real_content_hash_storage
        .get(&cache_key)
        .unwrap_or_default();
      Ok((value, real_content_hashes, Vec::new()))
    } else {
      let res = generator().await?;
      storage.set(cache_key, res.0.clone());
      real_content_hash_storage.set(cache_key, res.1.clone());
      Ok(res)
    }
  }
}
