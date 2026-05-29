use futures::Future;
use rspack_collections::Identifier;
use rspack_error::{Diagnostic, Result};
use rspack_sources::BoxSource;

use crate::{
  ArtifactExt, AssetHashRecord, Chunk, Compilation, MemoryGCStorage, SourceType,
  incremental::{Incremental, IncrementalPasses},
};

#[derive(Debug, Clone)]
pub struct ChunkRenderCacheValue {
  pub source: BoxSource,
  pub real_content_hashes: AssetHashRecord,
}

#[derive(Debug, Default)]
pub struct ChunkRenderCacheArtifact {
  storage: Option<MemoryGCStorage<ChunkRenderCacheValue>>,
}

impl ArtifactExt for ChunkRenderCacheArtifact {
  const PASS: IncrementalPasses = IncrementalPasses::CHUNK_ASSET;

  fn recover(_incremental: &Incremental, new: &mut Self, old: &mut Self) {
    *new = std::mem::take(old);
    new.start_next_generation();
  }
}

impl ChunkRenderCacheArtifact {
  pub fn new(storage: MemoryGCStorage<ChunkRenderCacheValue>) -> Self {
    Self {
      storage: Some(storage),
    }
  }
  pub fn start_next_generation(&self) {
    if let Some(storage) = &self.storage {
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
      Ok((value.source, Vec::new()))
    } else {
      let res = generator().await?;
      storage.set(
        cache_key,
        ChunkRenderCacheValue {
          source: res.0.clone(),
          real_content_hashes: AssetHashRecord::default(),
        },
      );
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
    let Some(content_hash) =
      chunk.content_hash_by_source_type(&compilation.chunk_hashes_artifact, source_type)
    else {
      return generator().await;
    };
    let cache_key = Identifier::from(content_hash.encoded());
    if let Some(value) = storage.get(&cache_key) {
      Ok((value.source, value.real_content_hashes, Vec::new()))
    } else {
      let res = generator().await?;
      storage.set(
        cache_key,
        ChunkRenderCacheValue {
          source: res.0.clone(),
          real_content_hashes: res.1.clone(),
        },
      );
      Ok(res)
    }
  }
}
