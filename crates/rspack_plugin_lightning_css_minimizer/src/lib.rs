use std::{
  collections::HashSet,
  hash::{Hash, Hasher},
  ops::Range,
  sync::{Arc, LazyLock, Mutex, RwLock},
};

pub use lightningcss::targets::Browsers;
use lightningcss::{
  printer::PrinterOptions,
  stylesheet::{MinifyOptions, ParserFlags, ParserOptions, StyleSheet},
  targets::{Features, Targets},
};
use rayon::prelude::*;
use regex::Regex;
use rspack_core::{
  AssetHashRecord, ChunkUkey, Compilation, CompilationChunkHash, CompilationProcessAssets,
  ContentHashReplacementKind, Plugin,
  diagnostics::MinifyError,
  rspack_sources::{
    BoxSource, MapOptions, ObjectPool, RawStringSource, ReplaceSource, Source, SourceExt,
    SourceMap, SourceMapSource, SourceMapSourceOptions,
  },
};
use rspack_error::{Diagnostic, Result, ToStringResultToRspackResultExt};
use rspack_hash::RspackHash;
use rspack_hook::{plugin, plugin_hook};
use rspack_util::{
  asset_condition::{AssetConditions, AssetConditionsObject, match_object},
  fx_hash::{FxHashMap, FxHasher},
};
use thread_local::ThreadLocal;

static CSS_ASSET_REGEXP: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"\.css(\?.*)?$").expect("Invalid RegExp"));

#[derive(Debug, Hash)]
pub struct PluginOptions {
  pub test: Option<AssetConditions>,
  pub include: Option<AssetConditions>,
  pub exclude: Option<AssetConditions>,
  pub remove_unused_local_idents: bool,
  pub minimizer_options: MinimizerOptions,
}

#[derive(Debug, Hash)]
pub struct Draft {
  pub custom_media: bool,
}

#[derive(Debug, Hash)]
pub struct NonStandard {
  pub deep_selector_combinator: bool,
}

#[derive(Debug, Hash)]
pub struct PseudoClasses {
  pub hover: Option<String>,
  pub active: Option<String>,
  pub focus: Option<String>,
  pub focus_visible: Option<String>,
  pub focus_within: Option<String>,
}

#[derive(Debug)]
pub struct MinimizerOptions {
  pub error_recovery: bool,
  pub targets: Option<Browsers>,
  pub include: Option<u32>,
  pub exclude: Option<u32>,
  pub drafts: Option<Draft>,
  pub non_standard: Option<NonStandard>,
  pub pseudo_classes: Option<PseudoClasses>,
  pub unused_symbols: Vec<String>,
}

impl Hash for MinimizerOptions {
  fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
    self.error_recovery.hash(state);
    self.include.hash(state);
    self.exclude.hash(state);
    self.drafts.hash(state);
    self.non_standard.hash(state);
    self.unused_symbols.hash(state);
    if let Some(pseudo_classes) = &self.pseudo_classes {
      pseudo_classes.hover.hash(state);
      pseudo_classes.active.hash(state);
      pseudo_classes.focus.hash(state);
      pseudo_classes.focus_visible.hash(state);
      pseudo_classes.focus_within.hash(state);
    }
    if let Some(targets) = &self.targets {
      targets.android.hash(state);
      targets.chrome.hash(state);
      targets.edge.hash(state);
      targets.firefox.hash(state);
      targets.ie.hash(state);
      targets.ios_saf.hash(state);
      targets.opera.hash(state);
      targets.safari.hash(state);
      targets.samsung.hash(state);
    }
  }
}

#[plugin]
#[derive(Debug)]
pub struct LightningCssMinimizerRspackPlugin {
  options: PluginOptions,
}

impl LightningCssMinimizerRspackPlugin {
  pub fn new(options: PluginOptions) -> Self {
    Self::new_inner(options)
  }
}

#[plugin_hook(CompilationChunkHash for LightningCssMinimizerRspackPlugin)]
async fn chunk_hash(
  &self,
  _compilation: &Compilation,
  _chunk_ukey: &ChunkUkey,
  hasher: &mut RspackHash,
) -> Result<()> {
  self.options.hash(hasher);
  Ok(())
}

#[plugin_hook(CompilationProcessAssets for LightningCssMinimizerRspackPlugin, stage = Compilation::PROCESS_ASSETS_STAGE_OPTIMIZE_SIZE)]
async fn process_assets(&self, compilation: &mut Compilation) -> Result<()> {
  let options = &self.options;
  let minimizer_options = &self.options.minimizer_options;
  let all_warnings: RwLock<Vec<Diagnostic>> = Default::default();
  let real_content_hash_records = compilation
    .options
    .optimization
    .real_content_hash
    .then(|| compilation.real_content_hash_artifact.asset_records.clone());
  let updated_real_content_hash_records = Mutex::new(FxHashMap::default());
  let condition_object = AssetConditionsObject {
    test: options.test.as_ref(),
    include: options.include.as_ref(),
    exclude: options.exclude.as_ref(),
  };

  let tls: ThreadLocal<ObjectPool> = ThreadLocal::new();
  compilation
    .assets_mut()
    .par_iter_mut()
    .filter(|(filename, original)| {
      if !CSS_ASSET_REGEXP.is_match(filename) {
        return false;
      }

      let is_matched = match_object(&condition_object, filename);

      if !is_matched || original.get_info().minimized.unwrap_or(false) {
        return false;
      }

      true
    })
    .try_for_each(|(filename, original)| -> Result<()> {
      if original.get_info().minimized.unwrap_or(false) {
        return Ok(());
      }

      if let Some(original_source) = original.get_source() {
        let input = original_source.source().into_string_lossy().into_owned();
        let original_input = input.clone();
        let record = real_content_hash_records
          .as_ref()
          .and_then(|records| records.get(filename));
        let (input, real_content_hash_markers) =
          mark_real_content_hash_replacements(record, input, filename)?;
        let object_pool = tls.get_or(ObjectPool::default);
        let input_source_map = original_source.map(object_pool, &MapOptions::default());

        let mut parser_flags = ParserFlags::empty();
        parser_flags.set(
          ParserFlags::CUSTOM_MEDIA,
          matches!(&minimizer_options.drafts, Some(drafts) if drafts.custom_media),
        );
        parser_flags.set(
          ParserFlags::DEEP_SELECTOR_COMBINATOR,
          matches!(&minimizer_options.non_standard, Some(non_standard) if non_standard.deep_selector_combinator),
        );

        let mut source_map = input_source_map
          .as_ref()
          .map(|input_source_map| -> Result<_> {
            let mut sm =
              parcel_sourcemap::SourceMap::new(input_source_map.source_root().unwrap_or("/"));
            sm.add_source(filename);
            sm.set_source_content(0, &original_input).to_rspack_result()?;
            Ok(sm)
          })
          .transpose()?;
        let result = {
          let warnings: Arc<RwLock<Vec<_>>> = Default::default();
          let mut stylesheet = StyleSheet::parse(
            &input,
            ParserOptions {
              filename: filename.clone(),
              css_modules: None,
              source_index: 0,
              error_recovery: minimizer_options.error_recovery,
              warnings: Some(warnings.clone()),
              flags: parser_flags,
            },
          )
          .to_rspack_result()?;

          let targets = Targets {
            browsers: minimizer_options.targets,
            include: minimizer_options
              .include
              .as_ref()
              .map_or(Features::empty(), |include| Features::from_bits_truncate(*include)),
            exclude: minimizer_options
              .exclude
              .as_ref()
              .map_or(Features::empty(), |exclude| Features::from_bits_truncate(*exclude)),
          };
          let mut unused_symbols = HashSet::from_iter(minimizer_options.unused_symbols.clone());
          if self.options.remove_unused_local_idents
            && let Some(css_unused_idents) = original.info.css_unused_idents.take()
          {
            unused_symbols.extend(css_unused_idents.into_iter().map(String::from));
          }
          stylesheet
            .minify(MinifyOptions {
              targets,
              unused_symbols,
            })
            .to_rspack_result()?;
          // FIXME: Disable the warnings for now, cause it cause too much positive-negative warnings,
          // enable when we have a better way to handle it. let warnings = warnings.read().expect("should lock");
          // all_warnings.write().expect("should lock").extend(
          //   warnings.iter().map(|e| {
          //     if let Some(loc) = &e.loc {
          //       let rope = ropey::Rope::from_str(&input);
          //       let start = rope.line_to_byte(loc.line as usize) + loc.column as usize - 1;
          //       let end = start;
          //       Diagnostic::from(Box::new(Error::from_file(
          //         input.clone(),
          //         start,
          //         end,
          //         "LightningCSS minimize warning".to_string(),
          //         e.to_string(),
          //       )
          //       .with_severity(Severity::Warning)))
          //     } else {
          //       Diagnostic::warn("LightningCSS minimize warning".to_string(), e.to_string())
          //     }
          //   }),
          // );
          stylesheet
            .to_css(PrinterOptions {
              minify: true,
              source_map: source_map.as_mut(),
              project_root: None,
              targets,
              analyze_dependencies: None,
              pseudo_classes: minimizer_options.pseudo_classes
              .as_ref()
              .map(|pseudo_classes| lightningcss::stylesheet::PseudoClasses {
                hover: pseudo_classes.hover.as_deref(),
                active: pseudo_classes.active.as_deref(),
                focus: pseudo_classes.focus.as_deref(),
                focus_visible: pseudo_classes.focus_visible.as_deref(),
                focus_within: pseudo_classes.focus_within.as_deref(),
              }),
            })
            .to_rspack_result()?
        };

        let mut minimized_source = if let Some(mut source_map) = source_map {
          SourceMapSource::new(SourceMapSourceOptions {
            value: result.code,
            name: filename,
            source_map: SourceMap::from_json(
              &source_map
                .to_json(None)
                .to_rspack_result()?,
            )
            .expect("should be able to generate source-map"),
            original_source: Some(Arc::from(original_input)),
            inner_source_map: input_source_map,
            remove_original_source: true,
          })
          .boxed()
        } else {
          RawStringSource::from(result.code).boxed()
        };

        let mut updated_real_content_hash_record = None;
        if let Some(record) = record {
          let (restored_source, updated_record) =
            restore_real_content_hash_markers(minimized_source, record, &real_content_hash_markers)?;
          minimized_source = restored_source;
          updated_real_content_hash_record = updated_record;
        }

        original.set_source(Some(minimized_source));
        if let Some(record) = updated_real_content_hash_record {
          updated_real_content_hash_records
            .lock()
            .expect("updated_real_content_hash_records lock failed")
            .insert(filename.clone(), record);
        }
      }
      original.get_info_mut().minimized.replace(true);
      Ok(())
    }).map_err(MinifyError)?;

  compilation.extend_diagnostics(all_warnings.into_inner().expect("should lock"));
  if compilation.options.optimization.real_content_hash {
    for (filename, record) in updated_real_content_hash_records
      .into_inner()
      .expect("updated_real_content_hash_records lock failed")
    {
      if let Some(existing) = compilation
        .real_content_hash_artifact
        .asset_records
        .get_mut(&filename)
      {
        *existing = record;
      }
    }
  }

  Ok(())
}

#[derive(Debug)]
struct RealContentHashMarker {
  replacement_index: usize,
  old_hash: String,
  marker: String,
}

fn mark_real_content_hash_replacements(
  record: Option<&AssetHashRecord>,
  mut input: String,
  filename: &str,
) -> Result<(String, Vec<RealContentHashMarker>)> {
  let Some(record) = record else {
    return Ok((input, Vec::new()));
  };

  let mut markers = Vec::new();
  for (replacement_index, replacement) in record.replacements.iter().enumerate() {
    if !matches!(
      replacement.kind,
      ContentHashReplacementKind::Source | ContentHashReplacementKind::Custom
    ) {
      continue;
    }
    let Some(range) = &replacement.range else {
      return Err(rspack_error::error!(
        "InvalidRealContentHashReplacementCoverage: asset '{}' has a {:?} content hash replacement for '{}' without a source range before CSS minimization",
        filename,
        replacement.kind,
        replacement.old_hash
      ));
    };
    let (Ok(start), Ok(end)) = (usize::try_from(range.start), usize::try_from(range.end)) else {
      return Err(rspack_error::error!(
        "InvalidRealContentHashReplacementCoverage: asset '{}' has an invalid {:?} content hash replacement range for '{}' before CSS minimization",
        filename,
        replacement.kind,
        replacement.old_hash
      ));
    };
    if input.get(start..end) != Some(replacement.old_hash.as_str()) {
      return Err(rspack_error::error!(
        "InvalidRealContentHashReplacementCoverage: asset '{}' has a stale {:?} content hash replacement range for '{}' before CSS minimization",
        filename,
        replacement.kind,
        replacement.old_hash
      ));
    }
    let Some(marker) =
      real_content_hash_marker(&input, replacement_index, start..end, &replacement.old_hash)
    else {
      return Err(rspack_error::error!(
        "InvalidRealContentHashReplacementCoverage: asset '{}' cannot generate a unique {:?} content hash marker for '{}' before CSS minimization",
        filename,
        replacement.kind,
        replacement.old_hash
      ));
    };
    markers.push((
      start..end,
      RealContentHashMarker {
        replacement_index,
        old_hash: replacement.old_hash.clone(),
        marker,
      },
    ));
  }

  markers.sort_unstable_by(|(a, _), (b, _)| b.start.cmp(&a.start).then_with(|| b.end.cmp(&a.end)));
  for (range, marker) in &markers {
    input.replace_range(range.clone(), &marker.marker);
  }

  Ok((
    input,
    markers.into_iter().map(|(_, marker)| marker).collect(),
  ))
}

fn restore_real_content_hash_markers(
  source: BoxSource,
  record: &AssetHashRecord,
  markers: &[RealContentHashMarker],
) -> Result<(BoxSource, Option<AssetHashRecord>)> {
  if markers.is_empty() {
    return Ok((source, None));
  }

  let content = source.buffer();
  let mut marker_ranges = Vec::new();
  let mut removed_markers = Vec::new();
  for marker in markers {
    let ranges = find_marker_ranges(&content, marker.marker.as_bytes());
    if ranges.is_empty() {
      removed_markers.push(marker);
      continue;
    }
    for range in ranges {
      marker_ranges.push((range, marker));
    }
  }
  marker_ranges.sort_unstable_by_key(|(range, _)| (range.start, range.end));

  let mut replace_source = ReplaceSource::new(source);
  let mut updated = record.clone();
  for marker in removed_markers {
    if let Some(replacement) = updated.replacements.get(marker.replacement_index).cloned()
      && let Some(reference_index) = updated.references.iter().position(|reference| {
        reference.referenced_hash == replacement.old_hash
          && reference_replacement_kind(reference.kind) == Some(replacement.kind)
      })
    {
      updated.references.remove(reference_index);
    }
    if let Some(replacement) = updated.replacements.get_mut(marker.replacement_index) {
      replacement.range = None;
    }
  }
  let mut removed_bytes = 0usize;
  let mut updated_replacement_indices = vec![false; updated.replacements.len()];
  for (range, marker) in marker_ranges {
    replace_source.replace(
      u32::try_from(range.start).expect("marker range start should fit in u32"),
      u32::try_from(range.end).expect("marker range end should fit in u32"),
      marker.old_hash.clone(),
      None,
    );
    let final_start = range.start - removed_bytes;
    let final_end = final_start + marker.old_hash.len();
    let final_range = u32::try_from(final_start).expect("replacement range start should fit in u32")
      ..u32::try_from(final_end).expect("replacement range end should fit in u32");
    if updated_replacement_indices[marker.replacement_index] {
      if let Some(replacement) = updated.replacements.get(marker.replacement_index) {
        let mut duplicated_replacement = replacement.clone();
        duplicated_replacement.range = Some(final_range);
        updated.replacements.push(duplicated_replacement);
      }
    } else if let Some(replacement) = updated.replacements.get_mut(marker.replacement_index) {
      replacement.range = Some(final_range);
      updated_replacement_indices[marker.replacement_index] = true;
    }
    removed_bytes += marker.marker.len() - marker.old_hash.len();
  }
  updated
    .replacements
    .retain(|replacement| replacement.range.is_some());

  Ok((replace_source.boxed(), Some(updated)))
}

fn real_content_hash_marker(
  input: &str,
  index: usize,
  range: Range<usize>,
  old_hash: &str,
) -> Option<String> {
  const MARKER_ALPHABET: &[u8] =
    b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_$";

  if old_hash.is_empty() {
    return None;
  }

  for salt in 0..128 {
    let mut hasher = FxHasher::default();
    input.hash(&mut hasher);
    index.hash(&mut hasher);
    range.start.hash(&mut hasher);
    range.end.hash(&mut hasher);
    old_hash.hash(&mut hasher);
    salt.hash(&mut hasher);

    let mut marker = String::with_capacity(old_hash.len());
    let mut value = hasher.finish();
    while marker.len() < old_hash.len() {
      let alphabet_index = (value as usize) % MARKER_ALPHABET.len();
      marker.push(MARKER_ALPHABET[alphabet_index] as char);
      value /= MARKER_ALPHABET.len() as u64;
      if value == 0 {
        salt.hash(&mut hasher);
        marker.len().hash(&mut hasher);
        value = hasher.finish();
      }
    }
    if marker == old_hash {
      continue;
    }
    if !input.contains(&marker) {
      return Some(marker);
    }
  }
  None
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

fn find_marker_ranges(content: &[u8], marker: &[u8]) -> Vec<Range<usize>> {
  let mut ranges = Vec::new();
  if marker.is_empty() || marker.len() > content.len() {
    return ranges;
  }

  let mut index = 0;
  while index + marker.len() <= content.len() {
    if content[index..].starts_with(marker) {
      ranges.push(index..index + marker.len());
      index += marker.len();
      continue;
    }
    index += 1;
  }
  ranges
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
  use rspack_core::{
    AssetHashRecord, ContentHashReplacement, ContentHashReplacementKind,
    rspack_sources::{RawStringSource, Source, SourceExt},
  };

  use super::{mark_real_content_hash_replacements, restore_real_content_hash_markers};

  #[test]
  fn real_content_hash_marker_restore_records_duplicated_markers() {
    let input = ".a{background:url(hhhh)}".to_string();
    let mut record = AssetHashRecord::default();
    record.replacements.push(ContentHashReplacement {
      old_hash: "hhhh".to_string(),
      range: Some(18..22),
      kind: ContentHashReplacementKind::Source,
    });

    let (_, markers) = mark_real_content_hash_replacements(Some(&record), input, "asset.css")
      .expect("valid source range should markerize");
    assert_eq!(markers.len(), 1);
    let marked = format!(
      ".a{{background:url({})}}.b{{background:url({})}}",
      markers[0].marker, markers[0].marker
    );

    let (restored, updated_record) =
      restore_real_content_hash_markers(RawStringSource::from(marked).boxed(), &record, &markers)
        .expect("duplicated markers should restore");

    assert_eq!(
      restored.source().into_string_lossy(),
      ".a{background:url(hhhh)}.b{background:url(hhhh)}"
    );
    let updated_record = updated_record.expect("duplicated marker should keep the record");
    assert_eq!(updated_record.replacements.len(), 2);
    assert_eq!(updated_record.replacements[0].range, Some(18..22));
    assert_eq!(updated_record.replacements[1].range, Some(42..46));
  }

  #[test]
  fn real_content_hash_markerize_rejects_unavailable_unique_markers() {
    let input = "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_$".to_string();
    let mut record = AssetHashRecord::default();
    record.replacements.push(ContentHashReplacement {
      old_hash: "0".to_string(),
      range: Some(0..1),
      kind: ContentHashReplacementKind::Source,
    });

    let err = mark_real_content_hash_replacements(Some(&record), input, "asset.css")
      .expect_err("unavailable marker should fail before CSS minimization");

    assert!(
      err
        .to_string()
        .contains("cannot generate a unique Source content hash marker")
    );
  }
}

impl Plugin for LightningCssMinimizerRspackPlugin {
  fn name(&self) -> &'static str {
    "rspack.LightningCssMinimizerRspackPlugin"
  }

  fn apply(&self, ctx: &mut rspack_core::ApplyContext<'_>) -> Result<()> {
    ctx.compilation_hooks.chunk_hash.tap(chunk_hash::new(self));
    ctx
      .compilation_hooks
      .process_assets
      .tap(process_assets::new(self));
    Ok(())
  }
}
