use std::{
  collections::hash_map::Entry,
  hash::Hash,
  ops::{Deref, DerefMut},
  sync::atomic::AtomicU32,
};

use anymap::CloneAny;
use rspack_collections::IdentifierMap;
use rspack_hash::{HashDigest, HashFunction, HashSalt, RspackHash, RspackHashDigest};
use rspack_sources::{BoxSource, CachedSource, SourceMapSource};
use rspack_util::atom::Atom;
use rustc_hash::{FxHashMap as HashMap, FxHashSet};
use serde::Serialize;

use crate::{
  ArtifactExt, AssetInfo, BindingCell, ChunkInitFragments, ConcatenationScope, ModuleIdentifier,
  RuntimeGlobals, RuntimeSpec, RuntimeSpecMap, SourceType, incremental::IncrementalPasses,
};

#[derive(Clone, Debug)]
pub struct CodeGenerationDataUrl {
  inner: String,
}

impl CodeGenerationDataUrl {
  pub fn new(inner: String) -> Self {
    Self { inner }
  }

  pub fn inner(&self) -> &str {
    &self.inner
  }
}

// For performance, mark the js modules containing AUTO_PUBLIC_PATH_PLACEHOLDER
#[derive(Clone, Debug)]
pub struct CodeGenerationPublicPathAutoReplace(pub bool);

#[derive(Clone, Debug)]
pub struct URLStaticMode;

#[derive(Clone, Debug)]
pub struct CodeGenerationDataFilename {
  filename: String,
  public_path: String,
}

impl CodeGenerationDataFilename {
  pub fn new(filename: String, public_path: String) -> Self {
    Self {
      filename,
      public_path,
    }
  }

  pub fn filename(&self) -> &str {
    &self.filename
  }

  pub fn public_path(&self) -> &str {
    &self.public_path
  }
}

#[derive(Clone, Debug)]
pub struct CodeGenerationDataAssetInfo {
  inner: AssetInfo,
}

impl CodeGenerationDataAssetInfo {
  pub fn new(inner: AssetInfo) -> Self {
    Self { inner }
  }

  pub fn inner(&self) -> &AssetInfo {
    &self.inner
  }
}

#[derive(Clone, Debug)]
pub struct CodeGenerationDataTopLevelDeclarations {
  inner: FxHashSet<Atom>,
}

impl CodeGenerationDataTopLevelDeclarations {
  pub fn new(inner: FxHashSet<Atom>) -> Self {
    Self { inner }
  }

  pub fn inner(&self) -> &FxHashSet<Atom> {
    &self.inner
  }
}

#[derive(Clone, Debug)]
pub struct CodeGenerationExportsFinalNames {
  inner: HashMap<String, String>,
}

impl CodeGenerationExportsFinalNames {
  pub fn new(inner: HashMap<String, String>) -> Self {
    Self { inner }
  }

  pub fn inner(&self) -> &HashMap<String, String> {
    &self.inner
  }
}

#[derive(Debug, Default, Clone)]
pub struct CodeGenerationData {
  inner: anymap::Map<dyn CloneAny + Send + Sync>,
}

impl Deref for CodeGenerationData {
  type Target = anymap::Map<dyn CloneAny + Send + Sync>;

  fn deref(&self) -> &Self::Target {
    &self.inner
  }
}

impl DerefMut for CodeGenerationData {
  fn deref_mut(&mut self) -> &mut Self::Target {
    &mut self.inner
  }
}

#[derive(Debug, Default, Clone)]
pub struct CodeGenerationResult {
  pub inner: BindingCell<HashMap<SourceType, BoxSource>>,
  /// [definition in webpack](https://github.com/webpack/webpack/blob/4b4ca3bb53f36a5b8fc6bc1bd976ed7af161bd80/lib/Module.js#L75)
  pub data: CodeGenerationData,
  pub chunk_init_fragments: ChunkInitFragments,
  pub runtime_requirements: RuntimeGlobals,
  pub hash: Option<RspackHashDigest>,
  pub id: CodeGenResultId,
  pub concatenation_scope: Option<ConcatenationScope>,
}

impl CodeGenerationResult {
  pub fn with_javascript(mut self, generation_result: BoxSource) -> Self {
    self.inner.insert(SourceType::JavaScript, generation_result);
    self
  }

  pub fn inner(&self) -> &HashMap<SourceType, BoxSource> {
    &self.inner
  }

  pub fn get(&self, source_type: &SourceType) -> Option<&BoxSource> {
    self.inner.get(source_type)
  }

  pub fn add(&mut self, source_type: SourceType, generation_result: BoxSource) {
    let result = self.inner.insert(source_type, generation_result);
    debug_assert!(result.is_none());
  }

  pub fn set_hash(
    &mut self,
    hash_function: &HashFunction,
    hash_digest: &HashDigest,
    hash_salt: &HashSalt,
  ) {
    let mut hasher = RspackHash::with_salt(hash_function, hash_salt);
    for (source_type, source) in self.inner.as_ref() {
      source_type.hash(&mut hasher);
      hash_source_content(source, &mut hasher);
    }
    self.chunk_init_fragments.hash(&mut hasher);
    self.runtime_requirements.hash(&mut hasher);
    self.hash = Some(hasher.digest(hash_digest));
  }
}

/// Hash the *emitted bytes* of a code generation source, matching webpack's
/// `CodeGenerationResults.getHash` semantics.
///
/// `Source::hash` for `SourceMapSource` folds in the wrapped `source_map`
/// (including its `sourcesContent` snapshot of the loader-input file). For
/// multi-block sources like Vue SFCs that means changing one block shifts the
/// codegen hash of every sibling sub-module even when their emitted bytes are
/// identical, which causes HMR to ship unchanged modules in hot-update chunks
/// (https://github.com/web-infra-dev/rspack/issues/11635).
///
/// Other source types (`RawStringSource`, `OriginalSource`, etc.) already
/// hash only the buffer in their own `Hash` impl, so we delegate to
/// `Source::hash` to keep `CachedSource`'s memoised digest on the hot path.
fn hash_source_content<H: std::hash::Hasher>(source: &BoxSource, hasher: &mut H) {
  // `source.as_any()` would resolve through the `Box<dyn Source>: AsAny`
  // blanket impl and hand back the Box wrapper. Deref to `&dyn Source` first
  // so the underlying concrete type's `as_any` is dispatched via the vtable.
  let inner: &dyn rspack_sources::Source = source.as_ref();
  let any = inner.as_any();
  if let Some(cached) = any.downcast_ref::<CachedSource>() {
    let cached_inner: &dyn rspack_sources::Source = cached.inner().as_ref();
    if cached_inner.as_any().is::<SourceMapSource>() {
      source.buffer().hash(hasher);
      return;
    }
  } else if any.is::<SourceMapSource>() {
    source.buffer().hash(hasher);
    return;
  }
  source.hash(hasher);
}

impl CodeGenerationResult {
  /// Concatenated modules already encode the generated module bodies into
  /// `ConcatenatedModule::get_runtime_hash`, so we can reuse that digest here
  /// and only mix in codegen-specific metadata instead of hashing the large
  /// concatenated source again.
  pub fn set_hash_for_concatenated_module(
    &mut self,
    runtime_hash: &RspackHashDigest,
    hash_function: &HashFunction,
    hash_digest: &HashDigest,
    hash_salt: &HashSalt,
  ) {
    let mut hasher = RspackHash::with_salt(hash_function, hash_salt);
    runtime_hash.hash(&mut hasher);
    for source_type in self.inner.as_ref().keys() {
      source_type.hash(&mut hasher);
    }
    self.chunk_init_fragments.hash(&mut hasher);
    self.runtime_requirements.hash(&mut hasher);
    self.hash = Some(hasher.digest(hash_digest));
  }
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, Ord, PartialOrd, Serialize)]
pub struct CodeGenResultId(u32);

impl Default for CodeGenResultId {
  fn default() -> Self {
    Self(CODE_GEN_RESULT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
  }
}

pub static CODE_GEN_RESULT_ID: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Default, Clone)]
pub struct CodeGenerationResults {
  module_generation_result_map: HashMap<CodeGenResultId, BindingCell<CodeGenerationResult>>,
  map: IdentifierMap<RuntimeSpecMap<CodeGenResultId>>,
}

impl ArtifactExt for CodeGenerationResults {
  const PASS: IncrementalPasses = IncrementalPasses::MODULES_CODEGEN;
}

impl CodeGenerationResults {
  pub fn is_empty(&self) -> bool {
    self.module_generation_result_map.is_empty() && self.map.is_empty()
  }

  pub fn insert(
    &mut self,
    module_identifier: ModuleIdentifier,
    codegen_res: CodeGenerationResult,
    runtimes: impl IntoIterator<Item = RuntimeSpec>,
  ) {
    let codegen_res_id = codegen_res.id;
    self
      .module_generation_result_map
      .insert(codegen_res_id, BindingCell::from(codegen_res));
    for runtime in runtimes {
      self.add(module_identifier, runtime, codegen_res_id);
    }
  }

  pub fn remove(&mut self, module_identifier: &ModuleIdentifier) -> Option<()> {
    let runtime_map = self.map.remove(module_identifier)?;
    for result in runtime_map.values() {
      self.module_generation_result_map.remove(result)?;
    }
    Some(())
  }

  pub fn get(
    &self,
    module_identifier: &ModuleIdentifier,
    runtime: Option<&RuntimeSpec>,
  ) -> &BindingCell<CodeGenerationResult> {
    if let Some(entry) = self.map.get(module_identifier) {
      if let Some(runtime) = runtime {
        entry
          .get(runtime)
          .and_then(|m| {
            self.module_generation_result_map.get(m)
          })
          .unwrap_or_else(|| {
            panic!(
              "Failed to code generation result for {module_identifier} with runtime {runtime:?} \n {entry:?}"
            )
          })
      } else {
        if entry.size() > 1 {
          let mut values = entry.values();
          let results: FxHashSet<_> = entry.values().collect();
          if results.len() > 1 {
            panic!(
              "No unique code generation entry for unspecified runtime for {module_identifier} ",
            );
          }

          return values
            .next()
            .and_then(|m| self.module_generation_result_map.get(m))
            .unwrap_or_else(|| panic!("Expected value exists"));
        }

        entry
          .values()
          .next()
          .and_then(|m| self.module_generation_result_map.get(m))
          .unwrap_or_else(|| panic!("Expected value exists"))
      }
    } else {
      panic!(
        "No code generation entry for {} (existing entries: {:?})",
        module_identifier,
        self.map.keys().collect::<Vec<_>>()
      )
    }
  }

  /**
   * This API should be used carefully, it will return one of the code generation result,
   * make sure the module has the same code generation result for all runtimes.
   */
  pub fn get_one(
    &self,
    module_identifier: &ModuleIdentifier,
  ) -> &BindingCell<CodeGenerationResult> {
    self
      .map
      .get(module_identifier)
      .and_then(|entry| {
        entry
          .values()
          .next()
          .and_then(|m| self.module_generation_result_map.get(m))
      })
      .unwrap_or_else(|| panic!("No code generation result for {module_identifier}"))
  }

  pub fn get_mut(
    &mut self,
    module_identifier: &ModuleIdentifier,
    runtime: Option<&RuntimeSpec>,
  ) -> &mut BindingCell<CodeGenerationResult> {
    if let Some(entry) = self.map.get(module_identifier) {
      if let Some(runtime) = runtime {
        entry
          .get(runtime)
          .and_then(|m| {
            self.module_generation_result_map.get_mut(m)
          })
          .unwrap_or_else(|| {
            panic!(
              "Failed to code generation result for {module_identifier} with runtime {runtime:?} \n {entry:?}"
            )
          })
      } else {
        if entry.size() > 1 {
          let mut values = entry.values();
          let results: FxHashSet<_> = entry.values().collect();
          if results.len() > 1 {
            panic!(
              "No unique code generation entry for unspecified runtime for {module_identifier} ",
            );
          }

          return values
            .next()
            .and_then(|m| self.module_generation_result_map.get_mut(m))
            .unwrap_or_else(|| panic!("Expected value exists"));
        }

        entry
          .values()
          .next()
          .and_then(|m| self.module_generation_result_map.get_mut(m))
          .unwrap_or_else(|| panic!("Expected value exists"))
      }
    } else {
      panic!(
        "No code generation entry for {} (existing entries: {:?})",
        module_identifier,
        self.map.keys().collect::<Vec<_>>()
      )
    }
  }

  pub fn add(
    &mut self,
    module_identifier: ModuleIdentifier,
    runtime: RuntimeSpec,
    result: CodeGenResultId,
  ) {
    match self.map.entry(module_identifier) {
      Entry::Occupied(mut record) => {
        record.get_mut().set(runtime, result);
      }
      Entry::Vacant(record) => {
        let mut spec_map = RuntimeSpecMap::default();
        spec_map.set(runtime, result);
        record.insert(spec_map);
      }
    };
  }

  pub fn get_runtime_requirements(
    &self,
    module_identifier: &ModuleIdentifier,
    runtime: Option<&RuntimeSpec>,
  ) -> RuntimeGlobals {
    self.get(module_identifier, runtime).runtime_requirements
  }

  pub fn get_hash(
    &self,
    module_identifier: &ModuleIdentifier,
    runtime: Option<&RuntimeSpec>,
  ) -> Option<&RspackHashDigest> {
    let code_generation_result = self.get(module_identifier, runtime);

    code_generation_result.hash.as_ref()
  }

  pub fn inner(
    &self,
  ) -> (
    &IdentifierMap<RuntimeSpecMap<CodeGenResultId>>,
    &HashMap<CodeGenResultId, BindingCell<CodeGenerationResult>>,
  ) {
    (&self.map, &self.module_generation_result_map)
  }
}

#[derive(Debug)]
pub struct CodeGenerationJob {
  pub module: ModuleIdentifier,
  pub hash: RspackHashDigest,
  pub runtime: RuntimeSpec,
  pub runtimes: Vec<RuntimeSpec>,
  pub scope: Option<ConcatenationScope>,
}

#[cfg(test)]
mod tests {
  use std::hash::Hasher;

  use rspack_sources::{
    BoxSource, CachedSource, RawStringSource, SourceExt, SourceMap, SourceMapSource,
    WithoutOriginalOptions,
  };
  use rustc_hash::FxHasher;

  use super::hash_source_content;

  fn raw(value: &str) -> BoxSource {
    RawStringSource::from(value.to_string()).boxed()
  }

  fn source_map_source(value: &str, sources_content: &str) -> BoxSource {
    let map = SourceMap::from_json(&format!(
      r#"{{"version":3,"sources":["child.vue"],"sourcesContent":[{}],"names":[],"mappings":""}}"#,
      serde_json::Value::String(sources_content.to_string()),
    ))
    .unwrap();
    SourceMapSource::new(WithoutOriginalOptions {
      value: value.to_string(),
      name: "child.vue".to_string(),
      source_map: map,
    })
    .boxed()
  }

  fn hash(source: &BoxSource) -> u64 {
    let mut hasher = FxHasher::default();
    hash_source_content(source, &mut hasher);
    hasher.finish()
  }

  #[test]
  fn raw_source_is_routed_through_source_hash() {
    // For non-SourceMapSource we delegate to `Source::hash` (which already
    // hashes only the buffer + a tag), preserving `CachedSource`'s memoised
    // digest fast path.
    let a = raw("hello");
    let b = raw("hello");
    let c = raw("world");
    assert_eq!(hash(&a), hash(&b));
    assert_ne!(hash(&a), hash(&c));
  }

  #[test]
  fn source_map_source_hash_only_depends_on_emitted_bytes() {
    let baseline = source_map_source("hello", "<template>child:0</template>");
    let baseline_hash = hash(&baseline);
    // Same emitted bytes, *different* sources_content (mimicking the Vue SFC
    // case where the template block changes but the script-setup output does
    // not). Hash must be stable.
    let same_bytes_other_map = source_map_source("hello", "<template>child:9</template>");
    assert_eq!(hash(&same_bytes_other_map), baseline_hash);
    let different_bytes = source_map_source("world", "<template>child:0</template>");
    assert_ne!(hash(&different_bytes), baseline_hash);
  }

  #[test]
  fn cached_source_wrapping_source_map_source_takes_buffer_path() {
    let baseline =
      CachedSource::new(source_map_source("hello", "<template>child:0</template>")).boxed();
    let baseline_hash = hash(&baseline);
    let same_bytes =
      CachedSource::new(source_map_source("hello", "<template>child:9</template>")).boxed();
    assert_eq!(hash(&same_bytes), baseline_hash);
  }
}
