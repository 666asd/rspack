mod drive;

use std::{
  hash::{BuildHasherDefault, Hasher},
  sync::{Arc, LazyLock},
};

use atomic_refcell::AtomicRefCell;
use derive_more::Debug;
pub use drive::*;
use rayon::prelude::*;
use rspack_core::{
  AssetHashRecord, AssetInfo, BindingCell, Compilation, CompilationId, CompilationProcessAssets,
  ContentHashReplacementKind, Logger, Plugin, RealContentHashArtifact,
  rspack_sources::{BoxSource, ReplaceSource, SourceExt},
};
use rspack_error::{Result, ToStringResultToRspackResultExt};
use rspack_hash::RspackHash;
use rspack_hook::{plugin, plugin_hook};
use rspack_util::fx_hash::FxDashMap;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet, FxHasher};

type IndexSet<T> = indexmap::IndexSet<T, BuildHasherDefault<FxHasher>>;

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

  let start = logger.time("create ordered hashes");
  let assets_data: HashMap<&str, AssetData> = compilation
    .assets()
    .par_iter()
    .filter_map(|(name, asset)| {
      asset.get_source().map(|source| {
        (
          name.as_str(),
          AssetData::new(source.clone(), asset.get_info()),
        )
      })
    })
    .collect();

  let (ordered_hashes, mut hash_dependencies) =
    OrderedHashesBuilder::new(&compilation.real_content_hash_artifact).build();
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
        None,
      )?;
      let new_name = compute_new_name(
        name,
        &data,
        compilation
          .real_content_hash_artifact
          .asset_records
          .get(name),
        &hash_to_new_hash,
      )?;
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

      if artifact
        .hash_to_assets
        .get(&reference.referenced_hash)
        .is_none_or(|asset_names| asset_names.is_empty())
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
  without_own: Option<&str>,
) -> Result<BoxSource> {
  if let Some(record) = artifact.asset_records.get(name) {
    return match get_recorded_replacement_coverage(record, data, hash_to_new_hash, without_own) {
      RecordedReplacementCoverage::Complete(plan) => apply_recorded_replacements(
        data.old_source.clone(),
        record,
        &plan,
        &data.own_hashes,
        hash_to_new_hash,
        without_own,
      ),
      RecordedReplacementCoverage::NoReplacementNeeded => Ok(data.old_source.clone()),
      RecordedReplacementCoverage::Incomplete | RecordedReplacementCoverage::Invalid => {
        Err(rspack_error::error!(
          "InvalidRealContentHashReplacementCoverage: recorded real content hash source byte ranges for asset '{}' do not match the asset source, are duplicated, or are incomplete for the current hash view",
          name
        ))
      }
    };
  }

  if without_own.is_some_and(|hash| data.own_hashes.contains(hash)) {
    return Err(rspack_error::error!(
      "InvalidRealContentHashReplacementCoverage: asset '{}' needs own content hash replacement but has no real content hash artifact record",
      name
    ));
  }

  Ok(data.old_source.clone())
}

fn compute_new_name(
  name: &str,
  _data: &AssetData,
  record: Option<&AssetHashRecord>,
  hash_to_new_hash: &HashMap<String, String>,
) -> Result<Option<String>> {
  let Some(record) = record else {
    return Ok(None);
  };
  let mut ranges = record
    .replacements
    .iter()
    .filter(|replacement| replacement.kind == ContentHashReplacementKind::Filename)
    .filter_map(|replacement| {
      let new_hash = hash_to_new_hash.get(&replacement.old_hash)?;
      (new_hash != &replacement.old_hash).then_some((replacement, new_hash))
    })
    .map(|(replacement, new_hash)| {
      let range = replacement.range.as_ref().ok_or_else(|| {
        rspack_error::error!(
          "InvalidRealContentHashFilenameReplacementRange: asset '{}' has a filename content hash replacement for '{}' without a filename byte range",
          name,
          replacement.old_hash
        )
      })?;
      let start = usize::try_from(range.start).map_err(|_| {
        rspack_error::error!(
          "InvalidRealContentHashFilenameReplacementRange: asset '{}' has an invalid filename range for '{}'",
          name,
          replacement.old_hash
        )
      })?;
      let end = usize::try_from(range.end).map_err(|_| {
        rspack_error::error!(
          "InvalidRealContentHashFilenameReplacementRange: asset '{}' has an invalid filename range for '{}'",
          name,
          replacement.old_hash
        )
      })?;
      if start > end || name.get(start..end) != Some(replacement.old_hash.as_str()) {
        return Err(rspack_error::error!(
          "InvalidRealContentHashFilenameReplacementRange: asset '{}' has a filename range that does not match content hash '{}'",
          name,
          replacement.old_hash
        ));
      }
      Ok((start..end, new_hash.as_str()))
    })
    .collect::<Result<Vec<_>>>()?;

  if ranges.is_empty() {
    return Ok(None);
  }

  ranges.sort_unstable_by_key(|(range, _)| (range.start, range.end));

  let mut new_name = String::with_capacity(name.len());
  let mut cursor = 0usize;
  for (range, new_hash) in ranges {
    if cursor > range.start {
      return Err(rspack_error::error!(
        "InvalidRealContentHashFilenameReplacementRange: asset '{}' has overlapping filename content hash ranges",
        name
      ));
    }
    new_name.push_str(&name[cursor..range.start]);
    new_name.push_str(new_hash);
    cursor = range.end;
  }
  new_name.push_str(&name[cursor..]);

  Ok((new_name != name).then_some(new_name))
}

enum RecordedReplacementCoverage {
  Complete(RecordedReplacementPlan),
  Incomplete,
  Invalid,
  NoReplacementNeeded,
}

struct RecordedReplacementPlan {
  selected_replacements: HashSet<usize>,
}

fn get_recorded_replacement_coverage(
  record: &AssetHashRecord,
  data: &AssetData,
  hash_to_new_hash: &HashMap<String, String>,
  without_own: Option<&str>,
) -> RecordedReplacementCoverage {
  let content = data.old_source.buffer();
  let mut ranges = Vec::new();
  let mut selected_replacements = HashSet::default();

  for (_, replacement) in record
    .replacements
    .iter()
    .enumerate()
    .filter(|(_, replacement)| is_source_replacement_kind(replacement.kind))
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
    if start > end || content.get(start..end) != Some(replacement.old_hash.as_bytes()) {
      return RecordedReplacementCoverage::Invalid;
    }
    ranges.push(start..end);
  }

  ranges.sort_unstable_by_key(|range| (range.start, range.end));
  let mut previous_end = None;
  for range in ranges {
    if previous_end.is_some_and(|end| end > range.start) {
      return RecordedReplacementCoverage::Invalid;
    }
    previous_end = Some(range.end);
  }

  let mut required_references = Vec::new();
  for reference in &record.references {
    if !is_source_reference_kind(reference.kind)
      || !reference_applies_to_view(reference.owner_hash.as_deref(), without_own)
      || !hash_needs_update(
        &reference.referenced_hash,
        data,
        hash_to_new_hash,
        without_own,
      )
    {
      continue;
    }
    increment_replacement_count(
      &mut required_references,
      &reference.referenced_hash,
      reference_replacement_kind(reference.kind).expect("source reference kind should map"),
    );
  }

  if has_ambiguous_owner_scoped_references(record, &required_references, without_own) {
    return RecordedReplacementCoverage::Incomplete;
  }

  let mut covered_references = Vec::new();
  for (index, replacement) in record
    .replacements
    .iter()
    .enumerate()
    .filter(|(_, replacement)| is_source_replacement_kind(replacement.kind))
  {
    if data.own_hashes.contains(&replacement.old_hash) {
      if replacement_needs_update_with_own(
        &replacement.old_hash,
        &data.own_hashes,
        hash_to_new_hash,
        without_own.is_some_and(|hash| data.own_hashes.contains(hash)),
      ) {
        selected_replacements.insert(index);
      }
      continue;
    }

    let required_count = get_replacement_count(
      &required_references,
      &replacement.old_hash,
      replacement.kind,
    );
    let covered_count =
      get_replacement_count(&covered_references, &replacement.old_hash, replacement.kind);

    if covered_count < required_count {
      selected_replacements.insert(index);
      increment_replacement_count(
        &mut covered_references,
        &replacement.old_hash,
        replacement.kind,
      );
      continue;
    }

    let all_reference_count = count_matching_source_references(record, replacement);
    let changed = hash_to_new_hash
      .get(&replacement.old_hash)
      .is_some_and(|new_hash| new_hash != &replacement.old_hash);
    if changed
      && (all_reference_count == 0 || without_own.is_none() || all_reference_count <= covered_count)
    {
      return RecordedReplacementCoverage::Incomplete;
    }
  }

  if required_references
    .iter()
    .any(|(hash, kind, count)| get_replacement_count(&covered_references, hash, *kind) < *count)
  {
    return RecordedReplacementCoverage::Incomplete;
  }

  if selected_replacements.is_empty() && required_references.is_empty() {
    return RecordedReplacementCoverage::NoReplacementNeeded;
  }

  RecordedReplacementCoverage::Complete(RecordedReplacementPlan {
    selected_replacements,
  })
}

fn has_ambiguous_owner_scoped_references(
  record: &AssetHashRecord,
  required_references: &[(String, ContentHashReplacementKind, usize)],
  without_own: Option<&str>,
) -> bool {
  let Some(owner) = without_own else {
    return false;
  };

  required_references.iter().any(|(hash, kind, _)| {
    record.references.iter().any(|reference| {
      reference
        .owner_hash
        .as_deref()
        .is_some_and(|other| other != owner)
        && reference.referenced_hash == *hash
        && reference_replacement_kind(reference.kind) == Some(*kind)
    })
  })
}

fn increment_replacement_count(
  counts: &mut Vec<(String, ContentHashReplacementKind, usize)>,
  hash: &str,
  kind: ContentHashReplacementKind,
) {
  for (counted_hash, counted_kind, count) in counts.iter_mut() {
    if counted_hash == hash && *counted_kind == kind {
      *count += 1;
      return;
    }
  }

  counts.push((hash.to_string(), kind, 1));
}

fn get_replacement_count(
  counts: &[(String, ContentHashReplacementKind, usize)],
  hash: &str,
  kind: ContentHashReplacementKind,
) -> usize {
  for (counted_hash, counted_kind, count) in counts {
    if counted_hash == hash && *counted_kind == kind {
      return *count;
    }
  }
  0
}

fn count_matching_source_references(
  record: &AssetHashRecord,
  replacement: &rspack_core::ContentHashReplacement,
) -> usize {
  record
    .references
    .iter()
    .filter(|reference| {
      reference.referenced_hash == replacement.old_hash
        && reference_replacement_kind(reference.kind) == Some(replacement.kind)
    })
    .count()
}

fn reference_applies_to_view(owner_hash: Option<&str>, without_own: Option<&str>) -> bool {
  without_own.is_none_or(|hash| owner_hash.is_none_or(|owner| owner == hash))
}

fn hash_needs_update(
  hash: &str,
  data: &AssetData,
  hash_to_new_hash: &HashMap<String, String>,
  without_own: Option<&str>,
) -> bool {
  without_own.is_some_and(|owner| data.own_hashes.contains(owner) && data.own_hashes.contains(hash))
    || hash_to_new_hash
      .get(hash)
      .is_none_or(|new_hash| new_hash != hash)
}

fn replacement_needs_update_with_own(
  old_hash: &str,
  own_hashes: &HashSet<String>,
  hash_to_new_hash: &HashMap<String, String>,
  without_own: bool,
) -> bool {
  (without_own && own_hashes.contains(old_hash))
    || hash_to_new_hash
      .get(old_hash)
      .is_none_or(|new_hash| new_hash != old_hash)
}

fn is_source_reference_kind(kind: rspack_core::ContentHashReferenceKind) -> bool {
  matches!(
    kind,
    rspack_core::ContentHashReferenceKind::Source | rspack_core::ContentHashReferenceKind::Custom
  )
}

fn reference_replacement_kind(
  kind: rspack_core::ContentHashReferenceKind,
) -> Option<ContentHashReplacementKind> {
  match kind {
    rspack_core::ContentHashReferenceKind::Source => Some(ContentHashReplacementKind::Source),
    rspack_core::ContentHashReferenceKind::Custom => Some(ContentHashReplacementKind::Custom),
    rspack_core::ContentHashReferenceKind::Filename => None,
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
  plan: &RecordedReplacementPlan,
  own_hashes: &HashSet<String>,
  hash_to_new_hash: &HashMap<String, String>,
  without_own: Option<&str>,
) -> Result<BoxSource> {
  let mut replace_source = ReplaceSource::new(source);

  for (index, replacement) in record.replacements.iter().enumerate() {
    if !is_source_replacement_kind(replacement.kind) {
      continue;
    }
    if !plan.selected_replacements.contains(&index) {
      continue;
    }
    let Some(range) = &replacement.range else {
      continue;
    };
    let replacement_value = if without_own.is_some_and(|hash| own_hashes.contains(hash))
      && own_hashes.contains(&replacement.old_hash)
    {
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
  #[debug(skip)]
  old_source: BoxSource,
}

impl AssetData {
  pub fn new(source: BoxSource, info: &AssetInfo) -> Self {
    let own_hashes = info.content_hash.iter().cloned().collect();

    Self {
      own_hashes,
      old_source: source,
    }
  }
}

struct OrderedHashesBuilder<'a> {
  artifact: &'a RealContentHashArtifact,
}

impl<'a> OrderedHashesBuilder<'a> {
  pub fn new(artifact: &'a RealContentHashArtifact) -> Self {
    Self { artifact }
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
  use rspack_core::{
    AssetInfo, ContentHashReference, ContentHashReferenceKind, ContentHashReplacement,
    ContentHashReplacementKind, RealContentHashArtifact,
    rspack_sources::{BoxSource, RawBufferSource, Source, SourceExt},
  };

  use super::{
    AssetData, HashMap, HashSet, RecordedReplacementPlan, apply_recorded_replacements,
    compute_new_name, compute_new_source, validate_artifact_records,
  };

  fn source(value: &str) -> BoxSource {
    RawBufferSource::from(value.as_bytes()).boxed()
  }

  fn replacement_plan(indexes: impl IntoIterator<Item = usize>) -> RecordedReplacementPlan {
    RecordedReplacementPlan {
      selected_replacements: HashSet::from_iter(indexes),
    }
  }

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
      source("url: aaaa bbbb"),
      &record,
      &replacement_plan([0]),
      &HashSet::default(),
      &hash_to_new_hash,
      None,
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
      source("url: aaaa"),
      &record,
      &replacement_plan([0]),
      &HashSet::default(),
      &hash_to_new_hash,
      None,
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
      source("url: aaaa"),
      &record,
      &replacement_plan([0]),
      &own_hashes,
      &hash_to_new_hash,
      Some("aaaa"),
    )
    .expect("own replacement should apply");

    assert_eq!(source.source().into_string_lossy(), "url: ");
  }

  #[test]
  fn compute_new_source_removes_all_own_hashes_for_recorded_without_own_view() {
    let info = AssetInfo::default()
      .with_content_hashes(HashSet::from_iter(["aaaa".to_string(), "bbbb".to_string()]));
    let data = AssetData::new(source("own aaaa and bbbb"), &info);
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
      Some("aaaa"),
    )
    .expect("source update should succeed");

    assert_eq!(source.source().into_string_lossy(), "own  and ");
  }

  #[test]
  fn compute_new_source_rejects_missing_recorded_source_ranges() {
    let info = AssetInfo::default().with_content_hashes(HashSet::from_iter(["aaaa".to_string()]));
    let data = AssetData::new(source("own aaaa ref bbbb"), &info);
    let mut artifact = RealContentHashArtifact::default();
    artifact.record_asset_hashes("asset.js", ["aaaa".to_string()]);
    artifact
      .asset_records
      .get_mut("asset.js")
      .expect("asset record")
      .references
      .push(ContentHashReference {
        referenced_hash: "bbbb".to_string(),
        owner_hash: Some("aaaa".to_string()),
        referenced_chunk: None,
        referenced_source_type: None,
        kind: ContentHashReferenceKind::Source,
      });
    let hash_to_new_hash = HashMap::from_iter([
      ("aaaa".to_string(), "cccc".to_string()),
      ("bbbb".to_string(), "dddd".to_string()),
    ]);

    let error = compute_new_source(
      "asset.js",
      &data,
      &artifact,
      &hash_to_new_hash,
      Some("aaaa"),
    )
    .expect_err("missing source records should be rejected");

    assert!(
      error
        .to_string()
        .contains("InvalidRealContentHashReplacementCoverage")
    );
  }

  #[test]
  fn compute_new_source_skips_owner_scoped_reference_for_unrelated_without_own_view() {
    let info = AssetInfo::default()
      .with_content_hashes(HashSet::from_iter(["aaaa".to_string(), "cccc".to_string()]));
    let data = AssetData::new(source("own aaaa and cccc ref bbbb"), &info);
    let mut artifact = RealContentHashArtifact::default();
    artifact.record_asset_hashes("asset.js", ["aaaa".to_string(), "cccc".to_string()]);
    let record = artifact
      .asset_records
      .get_mut("asset.js")
      .expect("asset record");
    record.references.push(ContentHashReference {
      referenced_hash: "bbbb".to_string(),
      owner_hash: Some("aaaa".to_string()),
      referenced_chunk: None,
      referenced_source_type: None,
      kind: ContentHashReferenceKind::Source,
    });
    record.replacements.push(ContentHashReplacement {
      old_hash: "aaaa".to_string(),
      range: Some(4..8),
      kind: ContentHashReplacementKind::Source,
    });
    record.replacements.push(ContentHashReplacement {
      old_hash: "cccc".to_string(),
      range: Some(13..17),
      kind: ContentHashReplacementKind::Source,
    });
    record.replacements.push(ContentHashReplacement {
      old_hash: "bbbb".to_string(),
      range: Some(22..26),
      kind: ContentHashReplacementKind::Source,
    });
    let hash_to_new_hash = HashMap::from_iter([
      ("aaaa".to_string(), "eeee".to_string()),
      ("cccc".to_string(), "ffff".to_string()),
    ]);

    let source = compute_new_source(
      "asset.js",
      &data,
      &artifact,
      &hash_to_new_hash,
      Some("cccc"),
    )
    .expect("unrelated owner-scoped reference should not be required");

    assert_eq!(source.source().into_string_lossy(), "own  and  ref bbbb");
  }

  #[test]
  fn compute_new_source_rejects_ambiguous_same_hash_owner_scoped_replacements() {
    let info = AssetInfo::default()
      .with_content_hashes(HashSet::from_iter(["aaaa".to_string(), "cccc".to_string()]));
    let data = AssetData::new(source("own aaaa and cccc refs hhhh then hhhh"), &info);
    let mut artifact = RealContentHashArtifact::default();
    artifact.record_asset_hashes("asset.js", ["aaaa".to_string(), "cccc".to_string()]);
    let record = artifact
      .asset_records
      .get_mut("asset.js")
      .expect("asset record");
    record.references.push(ContentHashReference {
      referenced_hash: "hhhh".to_string(),
      owner_hash: Some("aaaa".to_string()),
      referenced_chunk: None,
      referenced_source_type: None,
      kind: ContentHashReferenceKind::Source,
    });
    record.references.push(ContentHashReference {
      referenced_hash: "hhhh".to_string(),
      owner_hash: Some("cccc".to_string()),
      referenced_chunk: None,
      referenced_source_type: None,
      kind: ContentHashReferenceKind::Source,
    });
    record.replacements.push(ContentHashReplacement {
      old_hash: "aaaa".to_string(),
      range: Some(4..8),
      kind: ContentHashReplacementKind::Source,
    });
    record.replacements.push(ContentHashReplacement {
      old_hash: "cccc".to_string(),
      range: Some(13..17),
      kind: ContentHashReplacementKind::Source,
    });
    record.replacements.push(ContentHashReplacement {
      old_hash: "hhhh".to_string(),
      range: Some(23..27),
      kind: ContentHashReplacementKind::Source,
    });
    record.replacements.push(ContentHashReplacement {
      old_hash: "hhhh".to_string(),
      range: Some(33..37),
      kind: ContentHashReplacementKind::Source,
    });
    let hash_to_new_hash = HashMap::from_iter([
      ("aaaa".to_string(), "eeee".to_string()),
      ("cccc".to_string(), "ffff".to_string()),
      ("hhhh".to_string(), "iiii".to_string()),
    ]);

    let error = compute_new_source(
      "asset.js",
      &data,
      &artifact,
      &hash_to_new_hash,
      Some("cccc"),
    )
    .expect_err("ambiguous owner-scoped same-hash source records should be rejected");

    assert!(
      error
        .to_string()
        .contains("InvalidRealContentHashReplacementCoverage")
    );
  }

  #[test]
  fn compute_new_source_applies_distinct_owner_scoped_referenced_hashes() {
    let info = AssetInfo::default()
      .with_content_hashes(HashSet::from_iter(["aaaa".to_string(), "cccc".to_string()]));
    let data = AssetData::new(source("own aaaa and cccc refs bbbb then dddd"), &info);
    let mut artifact = RealContentHashArtifact::default();
    artifact.record_asset_hashes("asset.js", ["aaaa".to_string(), "cccc".to_string()]);
    let record = artifact
      .asset_records
      .get_mut("asset.js")
      .expect("asset record");
    record.references.push(ContentHashReference {
      referenced_hash: "bbbb".to_string(),
      owner_hash: Some("aaaa".to_string()),
      referenced_chunk: None,
      referenced_source_type: None,
      kind: ContentHashReferenceKind::Source,
    });
    record.references.push(ContentHashReference {
      referenced_hash: "dddd".to_string(),
      owner_hash: Some("cccc".to_string()),
      referenced_chunk: None,
      referenced_source_type: None,
      kind: ContentHashReferenceKind::Source,
    });
    record.replacements.push(ContentHashReplacement {
      old_hash: "aaaa".to_string(),
      range: Some(4..8),
      kind: ContentHashReplacementKind::Source,
    });
    record.replacements.push(ContentHashReplacement {
      old_hash: "cccc".to_string(),
      range: Some(13..17),
      kind: ContentHashReplacementKind::Source,
    });
    record.replacements.push(ContentHashReplacement {
      old_hash: "bbbb".to_string(),
      range: Some(23..27),
      kind: ContentHashReplacementKind::Source,
    });
    record.replacements.push(ContentHashReplacement {
      old_hash: "dddd".to_string(),
      range: Some(33..37),
      kind: ContentHashReplacementKind::Source,
    });
    let hash_to_new_hash = HashMap::from_iter([
      ("aaaa".to_string(), "eeee".to_string()),
      ("cccc".to_string(), "ffff".to_string()),
      ("bbbb".to_string(), "gggg".to_string()),
      ("dddd".to_string(), "iiii".to_string()),
    ]);

    let source = compute_new_source(
      "asset.js",
      &data,
      &artifact,
      &hash_to_new_hash,
      Some("cccc"),
    )
    .expect("distinct owner-scoped source records should be selectable");

    assert_eq!(
      source.source().into_string_lossy(),
      "own  and  refs bbbb then iiii"
    );
  }

  #[test]
  fn compute_new_source_ignores_filename_only_records() {
    let info = AssetInfo::default();
    let data = AssetData::new(source("ref aaaa"), &info);
    let mut artifact = RealContentHashArtifact::default();
    artifact.record_replacement(
      "asset.js",
      "aaaa",
      None,
      ContentHashReplacementKind::Filename,
    );
    let hash_to_new_hash = HashMap::from_iter([("aaaa".to_string(), "cccc".to_string())]);

    let source = compute_new_source("asset.js", &data, &artifact, &hash_to_new_hash, None)
      .expect("source update should succeed");

    assert_eq!(source.source().into_string_lossy(), "ref aaaa");
  }

  #[test]
  fn compute_new_name_replaces_recorded_filename_hashes_without_cascading() {
    let info = AssetInfo::default()
      .with_content_hashes(HashSet::from_iter(["aaaa".to_string(), "bbbb".to_string()]));
    let data = AssetData::new(source("asset"), &info);
    let mut record = rspack_core::AssetHashRecord::default();
    record.replacements.push(ContentHashReplacement {
      old_hash: "aaaa".to_string(),
      range: Some(6..10),
      kind: ContentHashReplacementKind::Filename,
    });
    record.replacements.push(ContentHashReplacement {
      old_hash: "bbbb".to_string(),
      range: Some(11..15),
      kind: ContentHashReplacementKind::Filename,
    });
    let hash_to_new_hash = HashMap::from_iter([
      ("aaaa".to_string(), "bbbb".to_string()),
      ("bbbb".to_string(), "cccc".to_string()),
    ]);

    let name = compute_new_name(
      "asset.aaaa.bbbb.js",
      &data,
      Some(&record),
      &hash_to_new_hash,
    )
    .expect("recorded filename ranges should be valid");

    assert_eq!(name.as_deref(), Some("asset.bbbb.cccc.js"));
  }

  #[test]
  fn compute_new_name_ignores_unrecorded_filename_substrings() {
    let info = AssetInfo::default();
    let data = AssetData::new(source("asset"), &info);
    let record = rspack_core::AssetHashRecord::default();
    let hash_to_new_hash = HashMap::from_iter([("aaaa".to_string(), "bbbb".to_string())]);

    let name = compute_new_name("asset.aaaa.js", &data, Some(&record), &hash_to_new_hash);

    assert_eq!(name.expect("empty record should be valid"), None);
  }

  #[test]
  fn compute_new_name_only_replaces_recorded_filename_ranges() {
    let info = AssetInfo::default();
    let data = AssetData::new(source("asset"), &info);
    let mut record = rspack_core::AssetHashRecord::default();
    record.replacements.push(ContentHashReplacement {
      old_hash: "aaaa".to_string(),
      range: Some(16..20),
      kind: ContentHashReplacementKind::Filename,
    });
    let hash_to_new_hash = HashMap::from_iter([("aaaa".to_string(), "bbbb".to_string())]);

    let name = compute_new_name(
      "asset.aaaa.copy.aaaa.js",
      &data,
      Some(&record),
      &hash_to_new_hash,
    )
    .expect("recorded filename range should be valid");

    assert_eq!(name.as_deref(), Some("asset.aaaa.copy.bbbb.js"));
  }

  #[test]
  fn compute_new_name_rejects_stale_filename_ranges() {
    let info = AssetInfo::default();
    let data = AssetData::new(source("asset"), &info);
    let mut record = rspack_core::AssetHashRecord::default();
    record.replacements.push(ContentHashReplacement {
      old_hash: "aaaa".to_string(),
      range: Some(0..4),
      kind: ContentHashReplacementKind::Filename,
    });
    let hash_to_new_hash = HashMap::from_iter([("aaaa".to_string(), "bbbb".to_string())]);

    let err = compute_new_name("asset.aaaa.js", &data, Some(&record), &hash_to_new_hash)
      .expect_err("stale filename range should fail");

    assert!(
      err
        .to_string()
        .contains("InvalidRealContentHashFilenameReplacementRange")
    );
  }

  #[test]
  fn compute_new_source_rejects_incomplete_recorded_source_ranges() {
    let info = AssetInfo::default().with_content_hashes(HashSet::from_iter(["aaaa".to_string()]));
    let data = AssetData::new(source("own aaaa ref bbbb"), &info);
    let mut artifact = RealContentHashArtifact::default();
    artifact.record_asset_hashes("asset.js", ["aaaa".to_string()]);
    artifact
      .asset_records
      .get_mut("asset.js")
      .expect("asset record")
      .references
      .push(ContentHashReference {
        referenced_hash: "bbbb".to_string(),
        owner_hash: Some("aaaa".to_string()),
        referenced_chunk: None,
        referenced_source_type: None,
        kind: ContentHashReferenceKind::Source,
      });
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

    let error = compute_new_source(
      "asset.js",
      &data,
      &artifact,
      &hash_to_new_hash,
      Some("aaaa"),
    )
    .expect_err("incomplete source records should be rejected");

    assert!(
      error
        .to_string()
        .contains("InvalidRealContentHashReplacementCoverage")
    );
  }

  #[test]
  fn compute_new_source_rejects_incomplete_duplicate_hash_ranges() {
    let info = AssetInfo::default();
    let data = AssetData::new(source("ref aaaa again aaaa"), &info);
    let mut artifact = RealContentHashArtifact::default();
    let record = artifact
      .asset_records
      .entry("asset.js".to_string())
      .or_default();
    record.references.push(ContentHashReference {
      referenced_hash: "aaaa".to_string(),
      owner_hash: None,
      referenced_chunk: None,
      referenced_source_type: None,
      kind: ContentHashReferenceKind::Source,
    });
    record.references.push(ContentHashReference {
      referenced_hash: "aaaa".to_string(),
      owner_hash: None,
      referenced_chunk: None,
      referenced_source_type: None,
      kind: ContentHashReferenceKind::Source,
    });
    record.replacements.push(ContentHashReplacement {
      old_hash: "aaaa".to_string(),
      range: Some(4..8),
      kind: ContentHashReplacementKind::Source,
    });
    let hash_to_new_hash = HashMap::from_iter([("aaaa".to_string(), "cccc".to_string())]);

    let error = compute_new_source("asset.js", &data, &artifact, &hash_to_new_hash, None)
      .expect_err("incomplete duplicate source records should be rejected");

    assert!(
      error
        .to_string()
        .contains("InvalidRealContentHashReplacementCoverage")
    );
  }

  #[test]
  fn compute_new_source_returns_old_source_when_recorded_ranges_do_not_need_replacement() {
    let info = AssetInfo::default();
    let data = AssetData::new(source("ref aaaa"), &info);
    let mut artifact = RealContentHashArtifact::default();
    artifact.record_replacement(
      "asset.js",
      "aaaa",
      Some(4..8),
      ContentHashReplacementKind::Source,
    );
    let hash_to_new_hash = HashMap::from_iter([("aaaa".to_string(), "aaaa".to_string())]);

    let source = compute_new_source("asset.js", &data, &artifact, &hash_to_new_hash, None)
      .expect("unchanged source records should be a no-op");

    assert_eq!(source.source().into_string_lossy(), "ref aaaa");
  }

  #[test]
  fn compute_new_source_rejects_overlapping_duplicate_replacement_ranges() {
    let info = AssetInfo::default();
    let data = AssetData::new(source("ref aaaa again aaaa"), &info);
    let mut artifact = RealContentHashArtifact::default();
    let record = artifact
      .asset_records
      .entry("asset.js".to_string())
      .or_default();
    record.references.push(ContentHashReference {
      referenced_hash: "aaaa".to_string(),
      owner_hash: None,
      referenced_chunk: None,
      referenced_source_type: None,
      kind: ContentHashReferenceKind::Source,
    });
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

    let error = compute_new_source("asset.js", &data, &artifact, &hash_to_new_hash, None)
      .expect_err("overlapping source records should be rejected");

    assert!(
      error
        .to_string()
        .contains("InvalidRealContentHashReplacementCoverage")
    );
  }

  #[test]
  fn compute_new_source_rejects_duplicate_replacement_ranges_for_single_occurrence() {
    let info = AssetInfo::default();
    let data = AssetData::new(source("ref aaaa"), &info);
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

    let error = compute_new_source("asset.js", &data, &artifact, &hash_to_new_hash, None)
      .expect_err("duplicate source records should be rejected");

    assert!(
      error
        .to_string()
        .contains("InvalidRealContentHashReplacementCoverage")
    );
  }

  #[test]
  fn compute_new_source_ignores_unchanged_recorded_ranges() {
    let info = AssetInfo::default();
    let data = AssetData::new(source("ref aaaa extra bbbb"), &info);
    let mut artifact = RealContentHashArtifact::default();
    let record = artifact
      .asset_records
      .entry("asset.js".to_string())
      .or_default();
    record.references.push(ContentHashReference {
      referenced_hash: "aaaa".to_string(),
      owner_hash: None,
      referenced_chunk: None,
      referenced_source_type: None,
      kind: ContentHashReferenceKind::Source,
    });
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

    let source = compute_new_source("asset.js", &data, &artifact, &hash_to_new_hash, None)
      .expect("unchanged source range should be a no-op");

    assert_eq!(source.source().into_string_lossy(), "ref cccc extra bbbb");
  }

  #[test]
  fn compute_new_source_rejects_recorded_range_that_points_at_wrong_text() {
    let info = AssetInfo::default();
    let data = AssetData::new(source("ref aaaa end"), &info);
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

    let error = compute_new_source("asset.js", &data, &artifact, &hash_to_new_hash, None)
      .expect_err("wrong source range should be rejected");

    assert!(
      error
        .to_string()
        .contains("InvalidRealContentHashReplacementCoverage")
    );
  }
}
