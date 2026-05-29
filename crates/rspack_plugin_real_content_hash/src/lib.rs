mod drive;

use std::{
  hash::{BuildHasherDefault, Hasher},
  sync::{Arc, LazyLock},
};

use aho_corasick::{AhoCorasick, MatchKind};
use atomic_refcell::AtomicRefCell;
use derive_more::Debug;
pub use drive::*;
use once_cell::sync::OnceCell;
use rayon::prelude::*;
use regex::Regex;
use rspack_core::{
  AssetHashRecord, AssetInfo, BindingCell, Compilation, CompilationId, CompilationProcessAssets,
  ContentHashReplacementKind, Logger, Plugin, RealContentHashArtifact,
  rspack_sources::{BoxSource, RawStringSource, ReplaceSource, SourceExt, SourceValue},
};
use rspack_error::{Result, ToStringResultToRspackResultExt};
use rspack_hash::RspackHash;
use rspack_hook::{plugin, plugin_hook};
use rspack_util::fx_hash::FxDashMap;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet, FxHasher};

type IndexSet<T> = indexmap::IndexSet<T, BuildHasherDefault<FxHasher>>;

pub static QUOTE_META: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"[-\[\]\\/{}()*+?.^$|]").expect("Invalid regex"));

/// Safety with [atomic_refcell::AtomicRefCell]:
///
/// We should make sure that there's no read-write and write-write conflicts for each hook instance by looking up [RealContentHashPlugin::get_compilation_hooks_mut]
type ArcReadContentHashPluginHooks = Arc<AtomicRefCell<RealContentHashPluginHooks>>;

static COMPILATION_HOOKS_MAP: LazyLock<FxDashMap<CompilationId, ArcReadContentHashPluginHooks>> =
  LazyLock::new(Default::default);

#[plugin]
#[derive(Debug, Default)]
pub struct RealContentHashPlugin;

impl RealContentHashPlugin {
  pub fn get_compilation_hooks(id: CompilationId) -> ArcReadContentHashPluginHooks {
    if !COMPILATION_HOOKS_MAP.contains_key(&id) {
      COMPILATION_HOOKS_MAP.insert(id, Default::default());
    }
    COMPILATION_HOOKS_MAP
      .get(&id)
      .expect("should have js plugin drive")
      .clone()
  }

  pub fn get_compilation_hooks_mut(id: CompilationId) -> ArcReadContentHashPluginHooks {
    COMPILATION_HOOKS_MAP.entry(id).or_default().clone()
  }
}

#[plugin_hook(CompilationProcessAssets for RealContentHashPlugin, stage = Compilation::PROCESS_ASSETS_STAGE_OPTIMIZE_HASH)]
async fn process_assets(&self, compilation: &mut Compilation) -> Result<()> {
  inner_impl(compilation).await
}

impl Plugin for RealContentHashPlugin {
  fn name(&self) -> &'static str {
    "rspack.RealContentHashPlugin"
  }

  fn apply(&self, ctx: &mut rspack_core::ApplyContext<'_>) -> Result<()> {
    ctx
      .compilation_hooks
      .process_assets
      .tap(process_assets::new(self));
    Ok(())
  }

  fn clear_cache(&self, id: CompilationId) {
    COMPILATION_HOOKS_MAP.remove(&id);
  }
}

async fn inner_impl(compilation: &mut Compilation) -> Result<()> {
  validate_artifact(compilation)?;

  let logger = compilation.get_logger("rspack.RealContentHashPlugin");
  let start = logger.time("hash to asset names");
  let mut hash_to_asset_names: HashMap<&str, Vec<&str>> = HashMap::default();
  for (name, asset) in compilation
    .assets()
    .iter()
    .filter(|(_, asset)| asset.get_source().is_some())
  {
    // e.g. filename: '[contenthash:8]-[contenthash:6].js'
    for hash in &asset.info.content_hash {
      hash_to_asset_names
        .entry(hash)
        .and_modify(|names| names.push(name))
        .or_insert_with(|| vec![name]);
    }
  }
  logger.time_end(start);
  if hash_to_asset_names.is_empty() {
    return Ok(());
  }
  let start = logger.time("create hash regexp");
  // use LeftmostLongest here:
  // e.g. 4afc|4afcbe match xxx.4afcbe-4afc.js -> xxx.[4afc]be-[4afc].js
  //      4afcbe|4afc match xxx.4afcbe-4afc.js -> xxx.[4afcbe]-[4afc].js
  let hash_ac = AhoCorasick::builder()
    .match_kind(MatchKind::LeftmostLongest)
    .build(hash_to_asset_names.keys().map(|s| s.as_bytes()))
    .expect("Invalid patterns");
  logger.time_end(start);

  let start = logger.time("create ordered hashes");
  let assets_data: HashMap<&str, AssetData> = compilation
    .assets()
    .par_iter()
    .filter_map(|(name, asset)| {
      asset.get_source().map(|source| {
        (
          name.as_str(),
          AssetData::new(source.clone(), asset.get_info(), &hash_ac),
        )
      })
    })
    .collect();

  let (ordered_hashes, mut hash_dependencies) =
    OrderedHashesBuilder::new(&compilation.real_content_hash_artifact, &assets_data).build();
  let mut ordered_hashes_iter = ordered_hashes.into_iter();

  logger.time_end(start);

  let start = logger.time("old hash to new hash");
  let mut hash_to_new_hash = HashMap::default();

  let hooks = RealContentHashPlugin::get_compilation_hooks(compilation.id());

  let mut computed_hashes = HashSet::default();
  let mut top_task = ordered_hashes_iter.next();

  while let Some(top) = top_task {
    let mut batch = vec![top];
    top_task = None;

    for hash in ordered_hashes_iter.by_ref() {
      let Some(dependencies) = hash_dependencies.remove(hash.as_str()) else {
        top_task = Some(hash);
        break;
      };
      if dependencies.iter().all(|dep| computed_hashes.contains(dep)) {
        batch.push(hash);
      } else {
        top_task = Some(hash);
        break;
      }
    }

    let batch_source_tasks = batch
      .iter()
      .filter_map(|hash| {
        let assets_names = hash_to_asset_names.get(hash.as_str())?;
        let tasks = assets_names
          .iter()
          .filter_map(|name| {
            let data = assets_data.get(name)?;
            Some((hash.as_str(), *name, data))
          })
          .collect::<Vec<_>>();
        Some(tasks)
      })
      .flatten()
      .collect::<Vec<_>>();

    let batch_sources = batch_source_tasks
      .into_par_iter()
      .map(|(hash, name, data)| {
        let new_source = compute_new_source(
          name,
          data,
          &compilation.real_content_hash_artifact,
          &hash_to_new_hash,
          &hash_ac,
          Some(hash),
        )?;
        Ok(((hash, name), new_source))
      })
      .collect::<Result<HashMap<_, _>>>()?;

    let new_hashes = rspack_parallel::scope::<_, Result<_>>(|token| {
      batch
        .iter()
        .cloned()
        .filter_map(|old_hash| {
          let asset_names = hash_to_asset_names.remove(old_hash.as_str())?;
          Some((old_hash, asset_names))
        })
        .for_each(|(old_hash, asset_names)| {
          let s =
            unsafe { token.used((&hooks, &compilation, &batch_sources, old_hash, asset_names)) };
          s.spawn(
            |(hooks, compilation, batch_sources, old_hash, mut asset_names)| async move {
              asset_names.sort_unstable();
              let mut asset_contents = asset_names
                .iter()
                .filter_map(|name| batch_sources.get(&(old_hash.as_str(), name)))
                .cloned()
                .collect::<Vec<_>>();
              asset_contents.dedup();
              let updated_hash = hooks
                .borrow()
                .update_hash
                .call(compilation, &asset_contents, &old_hash)
                .await?;

              let new_hash = if let Some(new_hash) = updated_hash {
                new_hash
              } else {
                let mut hasher = RspackHash::from(&compilation.options.output);
                for asset_content in asset_contents {
                  hasher.write(&asset_content.buffer());
                }
                let new_hash = hasher.digest(&compilation.options.output.hash_digest);

                new_hash.rendered(old_hash.len()).to_string()
              };

              Ok((old_hash.clone(), new_hash))
            },
          );
        });
    })
    .await
    .into_iter()
    .map(|r| r.to_rspack_result())
    .collect::<Result<Vec<_>>>()?;

    for res in new_hashes {
      let (old_hash, new_hash) = res?;
      hash_to_new_hash.insert(old_hash, new_hash);
    }

    computed_hashes.extend(batch);
  }

  logger.time_end(start);

  let start = logger.time("collect hash updates");
  let updates: Vec<_> = assets_data
    .into_par_iter()
    .map(|(name, data)| {
      let new_source = compute_new_source(
        name,
        &data,
        &compilation.real_content_hash_artifact,
        &hash_to_new_hash,
        &hash_ac,
        None,
      )?;
      let mut new_name = String::with_capacity(name.len());
      hash_ac.replace_all_with(name, &mut new_name, |_, hash, dst| {
        let replace_to = hash_to_new_hash
          .get(hash)
          .expect("RealContentHashPlugin: should have new hash");
        dst.push_str(replace_to);
        true
      });
      let new_name = (name != new_name).then_some(new_name);
      Ok((name.to_owned(), new_source, new_name))
    })
    .collect::<Result<Vec<_>>>()?;
  logger.time_end(start);

  let start = logger.time("update assets");
  let mut asset_renames = Vec::with_capacity(updates.len());
  for (name, new_source, new_name) in updates {
    compilation.update_asset(&name, |_, old_info| {
      let new_hashes: HashSet<_> = old_info
        .content_hash
        .iter()
        .map(|old_hash| {
          hash_to_new_hash
            .get(old_hash.as_str())
            .expect("should have new hash")
            .to_owned()
        })
        .collect();
      let info_update = (*old_info).clone();
      Ok((
        new_source.clone(),
        BindingCell::from(info_update.with_content_hashes(new_hashes)),
      ))
    })?;
    if let Some(new_name) = new_name {
      asset_renames.push((name, new_name));
    }
  }

  compilation.par_rename_assets(asset_renames);

  logger.time_end(start);

  Ok(())
}

fn validate_artifact(compilation: &Compilation) -> Result<()> {
  for (name, asset) in compilation.assets().iter() {
    if asset.get_source().is_none() || asset.info.content_hash.is_empty() {
      continue;
    }

    let Some(record) = compilation
      .real_content_hash_artifact
      .asset_records
      .get(name)
    else {
      return Err(rspack_error::error!(
        "MissingRealContentHashRecord: asset '{}' has content hashes {:?} but no real content hash artifact record",
        name,
        asset.info.content_hash
      ));
    };

    for hash in &asset.info.content_hash {
      if !record.own_hashes.contains(hash) {
        return Err(rspack_error::error!(
          "MissingRealContentHashOwnHash: asset '{}' exposes content hash '{}' but the real content hash artifact does not own it",
          name,
          hash
        ));
      }

      if !compilation
        .real_content_hash_artifact
        .hash_to_assets
        .get(hash)
        .is_some_and(|asset_names| asset_names.contains(name))
      {
        return Err(rspack_error::error!(
          "MissingRealContentHashReverseIndex: asset '{}' owns content hash '{}' but the real content hash artifact reverse index does not include it",
          name,
          hash
        ));
      }
    }
  }

  validate_artifact_records(&compilation.real_content_hash_artifact)?;

  Ok(())
}

fn validate_artifact_records(artifact: &RealContentHashArtifact) -> Result<()> {
  for (hash, asset_names) in &artifact.hash_to_assets {
    if asset_names.is_empty() {
      return Err(rspack_error::error!(
        "InvalidRealContentHashReverseIndex: content hash '{}' has an empty real content hash artifact reverse index",
        hash
      ));
    }

    for asset_name in asset_names {
      let Some(record) = artifact.asset_records.get(asset_name) else {
        return Err(rspack_error::error!(
          "InvalidRealContentHashReverseIndex: content hash '{}' points to asset '{}' but the asset has no real content hash artifact record",
          hash,
          asset_name
        ));
      };

      if !record.own_hashes.contains(hash) {
        return Err(rspack_error::error!(
          "InvalidRealContentHashReverseIndex: content hash '{}' points to asset '{}' but the asset record does not own it",
          hash,
          asset_name
        ));
      }
    }
  }

  for (asset_name, record) in &artifact.asset_records {
    for hash in &record.own_hashes {
      if !artifact
        .hash_to_assets
        .get(hash)
        .is_some_and(|asset_names| !asset_names.is_empty() && asset_names.contains(asset_name))
      {
        return Err(rspack_error::error!(
          "MissingRealContentHashReverseIndex: asset '{}' owns content hash '{}' but the real content hash artifact reverse index does not include it",
          asset_name,
          hash
        ));
      }
    }

    for reference in &record.references {
      if let Some(owner_hash) = &reference.owner_hash
        && !record.own_hashes.contains(owner_hash)
      {
        return Err(rspack_error::error!(
          "InvalidRealContentHashReferenceOwnerHash: asset '{}' references content hash '{}' from owner hash '{}' but the asset record does not own that hash",
          asset_name,
          reference.referenced_hash,
          owner_hash
        ));
      }

      if !artifact
        .hash_to_assets
        .get(&reference.referenced_hash)
        .is_some_and(|asset_names| !asset_names.is_empty())
      {
        return Err(rspack_error::error!(
          "MissingRealContentHashReferenceOwner: asset '{}' references unknown content hash '{}'",
          asset_name,
          reference.referenced_hash
        ));
      }
    }

    for replacement in &record.replacements {
      if matches!(
        replacement.kind,
        ContentHashReplacementKind::Source | ContentHashReplacementKind::Custom
      ) && replacement.range.is_none()
      {
        return Err(rspack_error::error!(
          "MissingRealContentHashReplacementRange: asset '{}' has a {:?} content hash replacement for '{}' without a source range",
          asset_name,
          replacement.kind,
          replacement.old_hash
        ));
      }
    }
  }

  Ok(())
}

fn compute_new_source(
  name: &str,
  data: &AssetData,
  artifact: &RealContentHashArtifact,
  hash_to_new_hash: &HashMap<String, String>,
  hash_ac: &AhoCorasick,
  without_own: Option<&str>,
) -> Result<BoxSource> {
  if let Some(record) = artifact.asset_records.get(name)
    && has_recorded_source_replacements(record)
  {
    match get_recorded_replacement_coverage(record, data, hash_to_new_hash, without_own) {
      RecordedReplacementCoverage::Complete => {
        return apply_recorded_replacements(
          data.old_source.clone(),
          record,
          &data.own_hashes,
          hash_to_new_hash,
          without_own.is_some_and(|hash| data.own_hashes.contains(hash)),
        );
      }
      RecordedReplacementCoverage::NoReplacementNeeded => return Ok(data.old_source.clone()),
      RecordedReplacementCoverage::Incomplete => {}
      RecordedReplacementCoverage::Invalid => {
        return Err(rspack_error::error!(
          "InvalidRealContentHashReplacementCoverage: recorded real content hash source byte ranges for asset '{}' do not match the asset source, are duplicated, or are extra for the current hash view",
          name
        ));
      }
    }
  }

  Ok(data.compute_new_source(
    without_own.is_some_and(|hash| data.own_hashes.contains(hash)),
    hash_to_new_hash,
    hash_ac,
  ))
}

fn has_recorded_source_replacements(record: &AssetHashRecord) -> bool {
  record
    .replacements
    .iter()
    .any(|replacement| is_source_replacement_kind(replacement.kind))
}

enum RecordedReplacementCoverage {
  Complete,
  Incomplete,
  Invalid,
  NoReplacementNeeded,
}

fn get_recorded_replacement_coverage(
  record: &AssetHashRecord,
  data: &AssetData,
  hash_to_new_hash: &HashMap<String, String>,
  without_own: Option<&str>,
) -> RecordedReplacementCoverage {
  let mut needed_hashes = HashSet::default();
  if without_own.is_some_and(|hash| data.own_hashes.contains(hash)) {
    needed_hashes.extend(data.own_hashes.iter().map(String::as_str));
  } else {
    needed_hashes.extend(data.own_hashes.iter().filter_map(|hash| {
      hash_to_new_hash
        .get(hash)
        .is_some_and(|new_hash| new_hash != hash)
        .then_some(hash.as_str())
    }));
  }
  needed_hashes.extend(data.referenced_hashes.iter().filter_map(|hash| {
    hash_to_new_hash
      .get(hash)
      .is_some_and(|new_hash| new_hash != hash)
      .then_some(hash.as_str())
  }));

  if needed_hashes.is_empty() {
    return RecordedReplacementCoverage::NoReplacementNeeded;
  }

  let AssetDataContent::String(content) = &data.content else {
    return RecordedReplacementCoverage::Invalid;
  };

  let mut needed_occurrence_ranges = HashSet::default();
  for occurrence in &data.hash_occurrences {
    if needed_hashes.contains(occurrence.hash.as_str()) {
      needed_occurrence_ranges.insert(occurrence.range.clone());
    }
  }

  if needed_occurrence_ranges.is_empty() {
    return RecordedReplacementCoverage::NoReplacementNeeded;
  }

  let mut covered_occurrence_ranges = HashSet::default();
  for replacement in record
    .replacements
    .iter()
    .filter(|replacement| is_source_replacement_kind(replacement.kind))
  {
    let Some(range) = &replacement.range else {
      return RecordedReplacementCoverage::Invalid;
    };
    let Ok(start) = usize::try_from(range.start) else {
      return RecordedReplacementCoverage::Invalid;
    };
    let Ok(end) = usize::try_from(range.end) else {
      return RecordedReplacementCoverage::Invalid;
    };
    if start > end || content.get(start..end) != Some(replacement.old_hash.as_str()) {
      return RecordedReplacementCoverage::Invalid;
    }
    let range = start..end;
    if !needed_hashes.contains(replacement.old_hash.as_str())
      || !needed_occurrence_ranges.contains(&range)
      || !covered_occurrence_ranges.insert(range)
    {
      return RecordedReplacementCoverage::Invalid;
    }
  }

  if needed_occurrence_ranges
    .iter()
    .all(|range| covered_occurrence_ranges.contains(range))
  {
    RecordedReplacementCoverage::Complete
  } else {
    RecordedReplacementCoverage::Incomplete
  }
}

fn is_source_replacement_kind(kind: ContentHashReplacementKind) -> bool {
  matches!(
    kind,
    ContentHashReplacementKind::Source | ContentHashReplacementKind::Custom
  )
}

fn apply_recorded_replacements(
  source: BoxSource,
  record: &AssetHashRecord,
  own_hashes: &HashSet<String>,
  hash_to_new_hash: &HashMap<String, String>,
  without_own: bool,
) -> Result<BoxSource> {
  let mut replace_source = ReplaceSource::new(source);

  for replacement in &record.replacements {
    if !is_source_replacement_kind(replacement.kind) {
      continue;
    }
    let Some(range) = &replacement.range else {
      continue;
    };
    let replacement_value = if without_own && own_hashes.contains(&replacement.old_hash) {
      ""
    } else {
      hash_to_new_hash
        .get(&replacement.old_hash)
        .ok_or_else(|| {
          rspack_error::error!(
            "MissingRealContentHashReplacementHash: content hash '{}' has no computed real content hash for a recorded source range",
            replacement.old_hash
          )
        })?
        .as_str()
    };

    replace_source.replace(range.start, range.end, replacement_value.to_string(), None);
  }

  Ok(replace_source.boxed())
}

#[derive(Debug)]
struct AssetData {
  own_hashes: HashSet<String>,
  referenced_hashes: HashSet<String>,
  hash_occurrences: Vec<SourceHashOccurrence>,
  #[debug(skip)]
  old_source: BoxSource,
  #[debug(skip)]
  content: AssetDataContent,
  #[debug(skip)]
  new_source: OnceCell<BoxSource>,
  #[debug(skip)]
  new_source_without_own: OnceCell<BoxSource>,
}

#[derive(Debug)]
struct SourceHashOccurrence {
  hash: String,
  range: std::ops::Range<usize>,
}

#[derive(Debug)]
enum AssetDataContent {
  Buffer,
  String(String),
}

impl AssetData {
  pub fn new(source: BoxSource, info: &AssetInfo, hash_ac: &AhoCorasick) -> Self {
    let mut own_hashes = HashSet::default();
    let mut referenced_hashes = HashSet::default();
    let mut hash_occurrences = Vec::new();
    let content = if let SourceValue::String(content) = source.source() {
      for hash_match in hash_ac.find_iter(content.as_ref()) {
        let range = hash_match.range();
        let hash = &content[range.clone()];
        hash_occurrences.push(SourceHashOccurrence {
          hash: hash.to_string(),
          range,
        });
        if info.content_hash.contains(hash) {
          own_hashes.insert(hash.to_string());
          continue;
        }
        referenced_hashes.insert(hash.to_string());
      }
      AssetDataContent::String(content.into_owned())
    } else {
      AssetDataContent::Buffer
    };

    Self {
      own_hashes,
      referenced_hashes,
      hash_occurrences,
      old_source: source,
      content,
      new_source: OnceCell::new(),
      new_source_without_own: OnceCell::new(),
    }
  }

  pub fn compute_new_source(
    &self,
    without_own: bool,
    hash_to_new_hash: &HashMap<String, String>,
    hash_ac: &AhoCorasick,
  ) -> BoxSource {
    (if without_own {
      &self.new_source_without_own
    } else {
      &self.new_source
    })
    .get_or_init(|| {
      if let AssetDataContent::String(content) = &self.content
        && (!self.own_hashes.is_empty()
          || self
            .referenced_hashes
            .iter()
            .any(|hash| matches!(hash_to_new_hash.get(hash.as_str()), Some(h) if h != hash)))
      {
        let mut new_content = String::with_capacity(content.len());
        hash_ac.replace_all_with(content, &mut new_content, |_, hash, dst| {
          let replace_to = if without_own && self.own_hashes.contains(hash) {
            ""
          } else {
            hash_to_new_hash
              .get(hash)
              .expect("RealContentHashPlugin: should have new hash")
          };
          dst.push_str(replace_to);
          true
        });
        return RawStringSource::from(new_content).boxed();
      }
      self.old_source.clone()
    })
    .clone()
  }
}

struct OrderedHashesBuilder<'a> {
  artifact: &'a RealContentHashArtifact,
  assets_data: &'a HashMap<&'a str, AssetData>,
}

impl<'a> OrderedHashesBuilder<'a> {
  pub fn new(
    artifact: &'a RealContentHashArtifact,
    assets_data: &'a HashMap<&'a str, AssetData>,
  ) -> Self {
    Self {
      artifact,
      assets_data,
    }
  }

  pub fn build(&self) -> (IndexSet<String>, HashMap<String, HashSet<String>>) {
    let mut ordered_hashes = IndexSet::default();
    let mut hash_dependencies = HashMap::default();
    for hash in self.artifact.hash_to_assets.keys() {
      self.add_to_ordered_hashes(
        hash,
        &mut ordered_hashes,
        &mut HashSet::default(),
        &mut hash_dependencies,
      );
    }
    (ordered_hashes, hash_dependencies)
  }
}

impl OrderedHashesBuilder<'_> {
  fn get_hash_dependencies(&self, hash: &str) -> HashSet<String> {
    let mut hashes = HashSet::default();
    let Some(asset_names) = self.artifact.hash_to_assets.get(hash) else {
      return hashes;
    };

    for name in asset_names {
      if let Some(record) = self.artifact.asset_records.get(name) {
        for reference in &record.references {
          if reference
            .owner_hash
            .as_deref()
            .is_none_or(|owner| owner == hash)
          {
            hashes.insert(reference.referenced_hash.clone());
          }
        }
      }

      if let Some(asset_hash) = self.assets_data.get(name.as_str()) {
        // Transitional fallback: source references are still discovered by the
        // scan-based replacement path until artifact source records are emitted.
        if !asset_hash.own_hashes.contains(hash) {
          for hash in &asset_hash.own_hashes {
            hashes.insert(hash.clone());
          }
        }
        for hash in &asset_hash.referenced_hashes {
          hashes.insert(hash.clone());
        }
      }
    }
    hashes
  }

  fn add_to_ordered_hashes(
    &self,
    hash: &str,
    ordered_hashes: &mut IndexSet<String>,
    stack: &mut HashSet<String>,
    hash_dependencies: &mut HashMap<String, HashSet<String>>,
  ) {
    let deps = hash_dependencies
      .entry(hash.to_string())
      .or_insert_with(|| self.get_hash_dependencies(hash))
      .clone();
    stack.insert(hash.to_string());
    for dep in deps {
      if ordered_hashes.contains(dep.as_str()) {
        continue;
      }
      if stack.contains(&dep) {
        // Safety: all chunk-level hash will be collected in runtime chunk
        // so there shouldn't have circular hash dependency between chunks
        panic!("RealContentHashPlugin: circular hash dependency");
      }
      self.add_to_ordered_hashes(&dep, ordered_hashes, stack, hash_dependencies);
    }
    ordered_hashes.insert(hash.to_string());
    stack.remove(hash);
  }
}

#[cfg(test)]
mod tests {
  use aho_corasick::AhoCorasick;
  use rspack_core::{
    AssetInfo, ContentHashReference, ContentHashReferenceKind, ContentHashReplacement,
    ContentHashReplacementKind, RealContentHashArtifact,
    rspack_sources::{RawStringSource, Source, SourceExt},
  };

  use super::{
    AssetData, HashMap, HashSet, apply_recorded_replacements, compute_new_source,
    validate_artifact_records,
  };

  #[test]
  fn validate_artifact_records_rejects_reference_owner_hash_not_owned_by_asset() {
    let mut artifact = RealContentHashArtifact::default();
    artifact.record_asset_hashes("asset.js", ["owned".to_string()]);
    artifact.record_asset_hashes("referenced.js", ["referenced".to_string()]);
    artifact
      .asset_records
      .get_mut("asset.js")
      .expect("asset record")
      .references
      .push(ContentHashReference {
        referenced_hash: "referenced".to_string(),
        owner_hash: Some("typo".to_string()),
        referenced_chunk: None,
        referenced_source_type: None,
        kind: ContentHashReferenceKind::Source,
      });

    let error = validate_artifact_records(&artifact).expect_err("should reject invalid owner hash");

    assert!(
      error
        .to_string()
        .contains("InvalidRealContentHashReferenceOwnerHash")
    );
  }

  #[test]
  fn validate_artifact_records_rejects_source_replacement_without_range() {
    let mut artifact = RealContentHashArtifact::default();
    artifact.record_asset_hashes("asset.js", ["owned".to_string()]);
    artifact
      .asset_records
      .get_mut("asset.js")
      .expect("asset record")
      .replacements
      .push(ContentHashReplacement {
        old_hash: "owned".to_string(),
        range: None,
        kind: ContentHashReplacementKind::Source,
      });

    let error =
      validate_artifact_records(&artifact).expect_err("should reject missing source range");

    assert!(
      error
        .to_string()
        .contains("MissingRealContentHashReplacementRange")
    );
  }

  #[test]
  fn validate_artifact_records_rejects_custom_replacement_without_range() {
    let mut artifact = RealContentHashArtifact::default();
    artifact.record_asset_hashes("asset.js", ["owned".to_string()]);
    artifact
      .asset_records
      .get_mut("asset.js")
      .expect("asset record")
      .replacements
      .push(ContentHashReplacement {
        old_hash: "owned".to_string(),
        range: None,
        kind: ContentHashReplacementKind::Custom,
      });

    let error =
      validate_artifact_records(&artifact).expect_err("should reject missing custom range");

    assert!(
      error
        .to_string()
        .contains("MissingRealContentHashReplacementRange")
    );
  }

  #[test]
  fn apply_recorded_replacements_updates_ranges_and_ignores_filename_records_without_range() {
    let mut record = rspack_core::AssetHashRecord::default();
    record.replacements.push(ContentHashReplacement {
      old_hash: "aaaa".to_string(),
      range: Some(5..9),
      kind: ContentHashReplacementKind::Source,
    });
    record.replacements.push(ContentHashReplacement {
      old_hash: "bbbb".to_string(),
      range: None,
      kind: ContentHashReplacementKind::Filename,
    });
    let hash_to_new_hash = HashMap::from_iter([("aaaa".to_string(), "cccc".to_string())]);

    let source = apply_recorded_replacements(
      RawStringSource::from("url: aaaa bbbb").boxed(),
      &record,
      &HashSet::default(),
      &hash_to_new_hash,
      false,
    )
    .expect("range replacement should apply");

    assert_eq!(source.source().into_string_lossy(), "url: cccc bbbb");
  }

  #[test]
  fn apply_recorded_replacements_ignores_ranged_filename_records() {
    let mut record = rspack_core::AssetHashRecord::default();
    record.replacements.push(ContentHashReplacement {
      old_hash: "aaaa".to_string(),
      range: Some(5..9),
      kind: ContentHashReplacementKind::Filename,
    });
    let hash_to_new_hash = HashMap::from_iter([("aaaa".to_string(), "cccc".to_string())]);

    let source = apply_recorded_replacements(
      RawStringSource::from("url: aaaa").boxed(),
      &record,
      &HashSet::default(),
      &hash_to_new_hash,
      false,
    )
    .expect("filename replacement should be ignored for source");

    assert_eq!(source.source().into_string_lossy(), "url: aaaa");
  }

  #[test]
  fn apply_recorded_replacements_removes_matching_own_hash() {
    let mut record = rspack_core::AssetHashRecord::default();
    record.replacements.push(ContentHashReplacement {
      old_hash: "aaaa".to_string(),
      range: Some(5..9),
      kind: ContentHashReplacementKind::Source,
    });
    let hash_to_new_hash = HashMap::from_iter([("aaaa".to_string(), "cccc".to_string())]);
    let own_hashes = HashSet::from_iter(["aaaa".to_string()]);

    let source = apply_recorded_replacements(
      RawStringSource::from("url: aaaa").boxed(),
      &record,
      &own_hashes,
      &hash_to_new_hash,
      true,
    )
    .expect("own replacement should apply");

    assert_eq!(source.source().into_string_lossy(), "url: ");
  }

  #[test]
  fn compute_new_source_removes_all_own_hashes_for_recorded_without_own_view() {
    let hash_ac = AhoCorasick::new(["aaaa", "bbbb"]).expect("valid hashes");
    let info = AssetInfo::default()
      .with_content_hashes(HashSet::from_iter(["aaaa".to_string(), "bbbb".to_string()]));
    let data = AssetData::new(
      RawStringSource::from("own aaaa and bbbb").boxed(),
      &info,
      &hash_ac,
    );
    let mut artifact = RealContentHashArtifact::default();
    artifact.record_asset_hashes("asset.js", ["aaaa".to_string(), "bbbb".to_string()]);
    artifact.record_replacement(
      "asset.js",
      "aaaa",
      Some(4..8),
      ContentHashReplacementKind::Source,
    );
    artifact.record_replacement(
      "asset.js",
      "bbbb",
      Some(13..17),
      ContentHashReplacementKind::Source,
    );
    let hash_to_new_hash = HashMap::from_iter([("aaaa".to_string(), "cccc".to_string())]);

    let source = compute_new_source(
      "asset.js",
      &data,
      &artifact,
      &hash_to_new_hash,
      &hash_ac,
      Some("aaaa"),
    )
    .expect("source update should succeed");

    assert_eq!(source.source().into_string_lossy(), "own  and ");
  }

  #[test]
  fn compute_new_source_falls_back_to_scan_without_recorded_source_ranges() {
    let hash_ac = AhoCorasick::new(["aaaa", "bbbb"]).expect("valid hashes");
    let info = AssetInfo::default().with_content_hashes(HashSet::from_iter(["aaaa".to_string()]));
    let data = AssetData::new(
      RawStringSource::from("own aaaa ref bbbb").boxed(),
      &info,
      &hash_ac,
    );
    let artifact = RealContentHashArtifact::default();
    let hash_to_new_hash = HashMap::from_iter([
      ("aaaa".to_string(), "cccc".to_string()),
      ("bbbb".to_string(), "dddd".to_string()),
    ]);

    let source = compute_new_source(
      "asset.js",
      &data,
      &artifact,
      &hash_to_new_hash,
      &hash_ac,
      Some("aaaa"),
    )
    .expect("source update should succeed");

    assert_eq!(source.source().into_string_lossy(), "own  ref dddd");
  }

  #[test]
  fn compute_new_source_falls_back_to_scan_with_filename_only_records() {
    let hash_ac = AhoCorasick::new(["aaaa"]).expect("valid hashes");
    let info = AssetInfo::default();
    let data = AssetData::new(RawStringSource::from("ref aaaa").boxed(), &info, &hash_ac);
    let mut artifact = RealContentHashArtifact::default();
    artifact.record_replacement(
      "asset.js",
      "aaaa",
      None,
      ContentHashReplacementKind::Filename,
    );
    let hash_to_new_hash = HashMap::from_iter([("aaaa".to_string(), "cccc".to_string())]);

    let source = compute_new_source(
      "asset.js",
      &data,
      &artifact,
      &hash_to_new_hash,
      &hash_ac,
      None,
    )
    .expect("source update should succeed");

    assert_eq!(source.source().into_string_lossy(), "ref cccc");
  }

  #[test]
  fn compute_new_source_falls_back_to_scan_when_recorded_source_ranges_are_incomplete() {
    let hash_ac = AhoCorasick::new(["aaaa", "bbbb"]).expect("valid hashes");
    let info = AssetInfo::default().with_content_hashes(HashSet::from_iter(["aaaa".to_string()]));
    let data = AssetData::new(
      RawStringSource::from("own aaaa ref bbbb").boxed(),
      &info,
      &hash_ac,
    );
    let mut artifact = RealContentHashArtifact::default();
    artifact.record_asset_hashes("asset.js", ["aaaa".to_string()]);
    artifact.record_replacement(
      "asset.js",
      "aaaa",
      Some(4..8),
      ContentHashReplacementKind::Source,
    );
    let hash_to_new_hash = HashMap::from_iter([
      ("aaaa".to_string(), "cccc".to_string()),
      ("bbbb".to_string(), "dddd".to_string()),
    ]);

    let source = compute_new_source(
      "asset.js",
      &data,
      &artifact,
      &hash_to_new_hash,
      &hash_ac,
      Some("aaaa"),
    )
    .expect("incomplete source records should fall back to scan");

    assert_eq!(source.source().into_string_lossy(), "own  ref dddd");
  }

  #[test]
  fn compute_new_source_falls_back_to_scan_when_duplicate_hash_ranges_are_incomplete() {
    let hash_ac = AhoCorasick::new(["aaaa"]).expect("valid hashes");
    let info = AssetInfo::default();
    let data = AssetData::new(
      RawStringSource::from("ref aaaa again aaaa").boxed(),
      &info,
      &hash_ac,
    );
    let mut artifact = RealContentHashArtifact::default();
    artifact
      .asset_records
      .entry("asset.js".to_string())
      .or_default()
      .replacements
      .push(ContentHashReplacement {
        old_hash: "aaaa".to_string(),
        range: Some(4..8),
        kind: ContentHashReplacementKind::Source,
      });
    let hash_to_new_hash = HashMap::from_iter([("aaaa".to_string(), "cccc".to_string())]);

    let source = compute_new_source(
      "asset.js",
      &data,
      &artifact,
      &hash_to_new_hash,
      &hash_ac,
      None,
    )
    .expect("incomplete duplicate source records should fall back to scan");

    assert_eq!(source.source().into_string_lossy(), "ref cccc again cccc");
  }

  #[test]
  fn compute_new_source_returns_old_source_when_recorded_ranges_do_not_need_replacement() {
    let hash_ac = AhoCorasick::new(["aaaa"]).expect("valid hashes");
    let info = AssetInfo::default();
    let data = AssetData::new(RawStringSource::from("ref aaaa").boxed(), &info, &hash_ac);
    let mut artifact = RealContentHashArtifact::default();
    artifact.record_replacement(
      "asset.js",
      "aaaa",
      Some(4..8),
      ContentHashReplacementKind::Source,
    );
    let hash_to_new_hash = HashMap::from_iter([("aaaa".to_string(), "aaaa".to_string())]);

    let source = compute_new_source(
      "asset.js",
      &data,
      &artifact,
      &hash_to_new_hash,
      &hash_ac,
      None,
    )
    .expect("unchanged source records should be a no-op");

    assert_eq!(source.source().into_string_lossy(), "ref aaaa");
  }

  #[test]
  fn compute_new_source_rejects_overlapping_duplicate_replacement_ranges() {
    let hash_ac = AhoCorasick::new(["aaaa"]).expect("valid hashes");
    let info = AssetInfo::default();
    let data = AssetData::new(
      RawStringSource::from("ref aaaa again aaaa").boxed(),
      &info,
      &hash_ac,
    );
    let mut artifact = RealContentHashArtifact::default();
    let record = artifact
      .asset_records
      .entry("asset.js".to_string())
      .or_default();
    record.replacements.push(ContentHashReplacement {
      old_hash: "aaaa".to_string(),
      range: Some(4..8),
      kind: ContentHashReplacementKind::Source,
    });
    record.replacements.push(ContentHashReplacement {
      old_hash: "aaaa".to_string(),
      range: Some(4..8),
      kind: ContentHashReplacementKind::Source,
    });
    let hash_to_new_hash = HashMap::from_iter([("aaaa".to_string(), "cccc".to_string())]);

    let error = compute_new_source(
      "asset.js",
      &data,
      &artifact,
      &hash_to_new_hash,
      &hash_ac,
      None,
    )
    .expect_err("overlapping source records should be rejected");

    assert!(
      error
        .to_string()
        .contains("InvalidRealContentHashReplacementCoverage")
    );
  }

  #[test]
  fn compute_new_source_rejects_duplicate_replacement_ranges_for_single_occurrence() {
    let hash_ac = AhoCorasick::new(["aaaa"]).expect("valid hashes");
    let info = AssetInfo::default();
    let data = AssetData::new(RawStringSource::from("ref aaaa").boxed(), &info, &hash_ac);
    let mut artifact = RealContentHashArtifact::default();
    let record = artifact
      .asset_records
      .entry("asset.js".to_string())
      .or_default();
    record.replacements.push(ContentHashReplacement {
      old_hash: "aaaa".to_string(),
      range: Some(4..8),
      kind: ContentHashReplacementKind::Source,
    });
    record.replacements.push(ContentHashReplacement {
      old_hash: "aaaa".to_string(),
      range: Some(4..8),
      kind: ContentHashReplacementKind::Source,
    });
    let hash_to_new_hash = HashMap::from_iter([("aaaa".to_string(), "cccc".to_string())]);

    let error = compute_new_source(
      "asset.js",
      &data,
      &artifact,
      &hash_to_new_hash,
      &hash_ac,
      None,
    )
    .expect_err("duplicate source records should be rejected");

    assert!(
      error
        .to_string()
        .contains("InvalidRealContentHashReplacementCoverage")
    );
  }

  #[test]
  fn compute_new_source_rejects_recorded_range_that_is_extra_for_view() {
    let hash_ac = AhoCorasick::new(["aaaa", "bbbb"]).expect("valid hashes");
    let info = AssetInfo::default();
    let data = AssetData::new(
      RawStringSource::from("ref aaaa extra bbbb").boxed(),
      &info,
      &hash_ac,
    );
    let mut artifact = RealContentHashArtifact::default();
    let record = artifact
      .asset_records
      .entry("asset.js".to_string())
      .or_default();
    record.replacements.push(ContentHashReplacement {
      old_hash: "aaaa".to_string(),
      range: Some(4..8),
      kind: ContentHashReplacementKind::Source,
    });
    record.replacements.push(ContentHashReplacement {
      old_hash: "bbbb".to_string(),
      range: Some(15..19),
      kind: ContentHashReplacementKind::Source,
    });
    let hash_to_new_hash = HashMap::from_iter([
      ("aaaa".to_string(), "cccc".to_string()),
      ("bbbb".to_string(), "bbbb".to_string()),
    ]);

    let error = compute_new_source(
      "asset.js",
      &data,
      &artifact,
      &hash_to_new_hash,
      &hash_ac,
      None,
    )
    .expect_err("extra source range should be rejected for this view");

    assert!(
      error
        .to_string()
        .contains("InvalidRealContentHashReplacementCoverage")
    );
  }

  #[test]
  fn compute_new_source_rejects_recorded_range_that_points_at_wrong_text() {
    let hash_ac = AhoCorasick::new(["aaaa"]).expect("valid hashes");
    let info = AssetInfo::default();
    let data = AssetData::new(
      RawStringSource::from("ref aaaa end").boxed(),
      &info,
      &hash_ac,
    );
    let mut artifact = RealContentHashArtifact::default();
    artifact
      .asset_records
      .entry("asset.js".to_string())
      .or_default()
      .replacements
      .push(ContentHashReplacement {
        old_hash: "aaaa".to_string(),
        range: Some(0..4),
        kind: ContentHashReplacementKind::Source,
      });
    let hash_to_new_hash = HashMap::from_iter([("aaaa".to_string(), "cccc".to_string())]);

    let error = compute_new_source(
      "asset.js",
      &data,
      &artifact,
      &hash_to_new_hash,
      &hash_ac,
      None,
    )
    .expect_err("wrong source range should be rejected");

    assert!(
      error
        .to_string()
        .contains("InvalidRealContentHashReplacementCoverage")
    );
  }
}
