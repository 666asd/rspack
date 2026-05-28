use std::ops::{Deref, DerefMut};

use rustc_hash::FxHashMap;

use crate::{ArtifactExt, ChunkUkey, RuntimeGlobals, incremental::IncrementalPasses};

#[derive(Debug, Default, Clone)]
pub struct RuntimeProxyMetadata {
  pub module_proxy_requirements: RuntimeGlobals,
  pub runtime_module_requirements: RuntimeGlobals,
  pub has_custom_runtime_module: bool,
  pub needs_require_bridge: bool,
  pub write_bridge_fields: RuntimeGlobals,
}

impl RuntimeProxyMetadata {
  pub fn lexical_fields(&self) -> RuntimeGlobals {
    self
      .runtime_module_requirements
      .union(self.module_proxy_requirements)
      .union(self.write_bridge_fields)
  }

  pub fn proxy_fields(&self) -> RuntimeGlobals {
    if self.has_custom_runtime_module {
      self.runtime_module_requirements
    } else {
      self.module_proxy_requirements
    }
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
