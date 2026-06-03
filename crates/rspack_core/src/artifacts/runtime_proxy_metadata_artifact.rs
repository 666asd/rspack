use std::ops::{Deref, DerefMut};

use rustc_hash::FxHashMap;

use crate::{ArtifactExt, ChunkUkey, RuntimeGlobals, incremental::IncrementalPasses};

#[derive(Debug, Default, Clone)]
pub struct RuntimeProxyMetadata {
  pub module_proxy_requirements: RuntimeGlobals,
  pub runtime_module_requirements: RuntimeGlobals,
  pub context_setter_fields: RuntimeGlobals,
  pub hook_exposed_requirements: RuntimeGlobals,
}

impl RuntimeProxyMetadata {
  pub fn lexical_fields(&self) -> RuntimeGlobals {
    let mut fields = self.runtime_module_requirements;
    fields.insert(self.module_proxy_requirements);
    fields.insert(self.context_setter_fields);
    fields.insert(self.hook_exposed_requirements);
    fields
  }

  pub fn context_fields(&self) -> RuntimeGlobals {
    let mut fields = self.module_proxy_requirements;
    fields.insert(self.context_setter_fields);
    fields.insert(self.hook_exposed_requirements);
    fields
  }
}

#[derive(Debug, Default, Clone)]
pub struct RuntimeProxyMetadataArtifact(FxHashMap<ChunkUkey, RuntimeProxyMetadata>);

impl ArtifactExt for RuntimeProxyMetadataArtifact {
  const PASS: IncrementalPasses = IncrementalPasses::CHUNKS_RUNTIME_REQUIREMENTS;
}

impl Deref for RuntimeProxyMetadataArtifact {
  type Target = FxHashMap<ChunkUkey, RuntimeProxyMetadata>;

  fn deref(&self) -> &Self::Target {
    &self.0
  }
}

impl DerefMut for RuntimeProxyMetadataArtifact {
  fn deref_mut(&mut self) -> &mut Self::Target {
    &mut self.0
  }
}

impl From<FxHashMap<ChunkUkey, RuntimeProxyMetadata>> for RuntimeProxyMetadataArtifact {
  fn from(value: FxHashMap<ChunkUkey, RuntimeProxyMetadata>) -> Self {
    Self(value)
  }
}

impl From<RuntimeProxyMetadataArtifact> for FxHashMap<ChunkUkey, RuntimeProxyMetadata> {
  fn from(value: RuntimeProxyMetadataArtifact) -> Self {
    value.0
  }
}

impl FromIterator<<FxHashMap<ChunkUkey, RuntimeProxyMetadata> as IntoIterator>::Item>
  for RuntimeProxyMetadataArtifact
{
  fn from_iter<
    T: IntoIterator<Item = <FxHashMap<ChunkUkey, RuntimeProxyMetadata> as IntoIterator>::Item>,
  >(
    iter: T,
  ) -> Self {
    Self(FxHashMap::from_iter(iter))
  }
}

impl IntoIterator for RuntimeProxyMetadataArtifact {
  type Item = <FxHashMap<ChunkUkey, RuntimeProxyMetadata> as IntoIterator>::Item;
  type IntoIter = <FxHashMap<ChunkUkey, RuntimeProxyMetadata> as IntoIterator>::IntoIter;

  fn into_iter(self) -> Self::IntoIter {
    self.0.into_iter()
  }
}
