use std::{
  hash::{Hash, Hasher},
  ops::Range,
  path::Path,
  sync::{LazyLock, Mutex, mpsc},
};

use cow_utils::CowUtils;
use once_cell::sync::OnceCell;
use rayon::prelude::*;
use regex::Regex;
use rspack_core::{
  AssetHashRecord, AssetInfo, ChunkUkey, Compilation, CompilationAsset, CompilationParams,
  CompilationProcessAssets, CompilerCompilation, ContentHashReplacementKind, Logger, Plugin,
  cache::persistent::occasion::minimize::{
    CachedExtractedComments, CachedMinimizeEntry, MinimizeCacheKey,
  },
  diagnostics::MinifyError,
  rspack_sources::{
    BoxSource, ConcatSource, MapOptions, ObjectPool, RawStringSource, ReplaceSource, Source,
    SourceExt, SourceMapSource, SourceMapSourceOptions,
  },
};
use rspack_error::{Diagnostic, Result};
use rspack_hash::RspackHash;
use rspack_hook::{plugin, plugin_hook};
use rspack_javascript_compiler::JavaScriptCompiler;
use rspack_plugin_javascript::{ExtractedCommentsInfo, JavascriptModulesChunkHash, JsPlugin};
use rspack_regex::RspackRegex;
use rspack_util::{
  asset_condition::AssetConditions,
  fx_hash::{FxHashMap, FxHasher},
};
use swc_config::types::BoolOrDataConfig;
use swc_core::{
  base::config::JsMinifyFormatOptions,
  common::comments::{CommentKind, SingleThreadedComments},
};
pub use swc_ecma_minifier::option::{
  MangleOptions,
  terser::{TerserCompressorOptions, TerserEcmaVersion},
};
use thread_local::ThreadLocal;

const PLUGIN_NAME: &str = "rspack.SwcJsMinimizerRspackPlugin";

static JAVASCRIPT_ASSET_REGEXP: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"\.[cm]?js(\?.*)?$").expect("Invalid RegExp"));

#[derive(Debug, Hash)]
pub struct PluginOptions {
  pub test: Option<AssetConditions>,
  pub include: Option<AssetConditions>,
  pub exclude: Option<AssetConditions>,
  pub extract_comments: Option<ExtractComments>,
  pub minimizer_options: MinimizerOptions,
}

#[derive(Debug, Default)]
pub struct MinimizerOptions {
  pub ecma: TerserEcmaVersion,
  pub minify: Option<bool>,
  pub compress: BoolOrDataConfig<TerserCompressorOptions>,
  pub mangle: BoolOrDataConfig<MangleOptions>,
  pub format: JsMinifyFormatOptions,
  pub module: Option<bool>,

  /// Internal fields for hashing only.
  /// This guaranteed these field should only be readonly.
  /// Otherwise, hash would be generated with inconsistencies.
  pub __compress_cache: OnceCell<BoolOrDataConfig<String>>,
  pub __mangle_cache: OnceCell<BoolOrDataConfig<String>>,
  pub __format_cache: OnceCell<String>,
}

impl std::hash::Hash for MinimizerOptions {
  fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
    self
      .__format_cache
      .get_or_init(|| serde_json::to_string(&self.format).expect("Should be able to serialize"))
      .hash(state);
    self
      .__compress_cache
      .get_or_init(|| {
        self
          .compress
          .as_ref()
          .map(|v| serde_json::to_string(v).expect("Should be able to serialize"))
      })
      .hash(state);
    self
      .__mangle_cache
      .get_or_init(|| {
        self
          .mangle
          .as_ref()
          .map(|v| serde_json::to_string(v).expect("Should be able to serialize"))
      })
      .hash(state);
  }
}

#[derive(Debug, Hash)]
pub enum OptionWrapper<T: std::fmt::Debug + Hash> {
  Default,
  Disabled,
  Custom(T),
}

#[derive(Debug)]
pub struct ExtractComments {
  pub condition: String,
  pub condition_flags: String,
  pub banner: OptionWrapper<String>,
}

impl Hash for ExtractComments {
  fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
    self.condition.as_str().hash(state);
    self.condition_flags.as_str().hash(state);
    self.banner.hash(state);
  }
}

#[derive(Debug)]
struct NormalizedExtractComments {
  filename: String,
  condition: RspackRegex,
  banner: Option<String>,
}

#[plugin]
#[derive(Debug)]
pub struct SwcJsMinimizerRspackPlugin {
  options: PluginOptions,
  options_hash: u64,
}

impl SwcJsMinimizerRspackPlugin {
  pub fn new(options: PluginOptions) -> Self {
    let mut hasher = FxHasher::default();
    PLUGIN_NAME.hash(&mut hasher);
    options.hash(&mut hasher);
    let options_hash = hasher.finish();
    Self::new_inner(options, options_hash)
  }
}

#[plugin_hook(CompilerCompilation for SwcJsMinimizerRspackPlugin)]
async fn compilation(
  &self,
  compilation: &mut Compilation,
  _params: &mut CompilationParams,
) -> Result<()> {
  let hooks = JsPlugin::get_compilation_hooks_mut(compilation.id());
  let mut hooks = hooks.write().await;
  hooks.chunk_hash.tap(js_chunk_hash::new(self));
  Ok(())
}

#[plugin_hook(JavascriptModulesChunkHash for SwcJsMinimizerRspackPlugin)]
async fn js_chunk_hash(
  &self,
  _compilation: &Compilation,
  _chunk_ukey: &ChunkUkey,
  hasher: &mut RspackHash,
) -> Result<()> {
  self.options_hash.hash(hasher);
  Ok(())
}

#[plugin_hook(CompilationProcessAssets for SwcJsMinimizerRspackPlugin, stage = Compilation::PROCESS_ASSETS_STAGE_OPTIMIZE_SIZE)]
async fn process_assets(&self, compilation: &mut Compilation) -> Result<()> {
  let options = &self.options;
  let minimizer_options = &self.options.minimizer_options;

  // Take persistent cache out if enabled. When Some, we compute cache keys and
  // do lookups; when None, we skip all cache overhead entirely.
  let minimize_persistent_cache = compilation.minimize_persistent_cache_artifact.take();
  let new_persistent_cache_entries: Mutex<Vec<(MinimizeCacheKey, CachedMinimizeEntry)>> =
    Mutex::new(Vec::new());
  let logger = compilation.get_logger(PLUGIN_NAME);
  let minimize_cache_counter = minimize_persistent_cache
    .as_ref()
    .map(|_| logger.cache("minimize persistent cache"));
  let real_content_hash_records = compilation.real_content_hash_artifact.asset_records.clone();
  let updated_real_content_hash_records = Mutex::new(FxHashMap::default());

  let (tx, rx) = mpsc::channel::<Vec<Diagnostic>>();
  // collect all extracted comments info
  let all_extracted_comments = Mutex::new(FxHashMap::default());
  let extract_comments_condition = options.extract_comments.as_ref().map(|extract_comment| {
    RspackRegex::with_flags(
      extract_comment.condition.as_ref(),
      extract_comment.condition_flags.as_ref(),
    )
    .unwrap_or_else(|_| {
      panic!(
        "`/{}/{}` is invalid extractComments condition",
        extract_comment.condition, extract_comment.condition_flags
      )
    })
  });
  let enter_span = tracing::Span::current();

  let tls: ThreadLocal<ObjectPool> = ThreadLocal::new();
  compilation
    .assets_mut()
    .par_iter_mut()
    .filter(|(filename, original)| {
      // propagate span in rayon to keep parent relation
      let is_matched = match_object(options, filename);

      if !is_matched || original.get_info().minimized.unwrap_or(false) {
        return false
      }

      true
    })
    .try_for_each_with(tx,|tx, (filename, original)| -> Result<()>  {
      let _guard = enter_span.enter();
      let filename = filename.split('?').next().expect("Should have filename");
      if let Some(original_source) = original.get_source() {
        let is_module = if let Some(module) = minimizer_options.module {
          Some(module)
        } else if let Some(module) = original.info.javascript_module {
          Some(module)
        } else if filename.ends_with(".mjs") {
          Some(true)
        } else if filename.ends_with(".cjs") {
          Some(false)
        } else {
          None
        };

        let input = original_source.source().into_string_lossy().into_owned();
        let record = real_content_hash_records.get(filename);
        let (input, real_content_hash_markers) =
          mark_real_content_hash_replacements(record, input, filename)?;

        // Compute cache key and check persistent cache (only when enabled).
        // Marked inputs carry per-compilation range metadata, so skip the
        // persistent source cache for them.
        let cache_key = if real_content_hash_markers.is_empty()
          && let Some(cache) = &minimize_persistent_cache
        {
          let key = {
            let mut hasher = FxHasher::default();
            input.as_bytes().hash(&mut hasher);
            self.options_hash.hash(&mut hasher);
            filename.hash(&mut hasher);
            is_module.hash(&mut hasher);
            MinimizeCacheKey::new(hasher.finish())
          };

          // Check persistent cache
          if let Some(cached) = cache.get(key) {
            if let Some(counter) = &minimize_cache_counter {
              counter.hit();
            }
            original.set_source(Some(cached.source.clone()));
            original.get_info_mut().minimized.replace(true);
            if let Some(ec) = &cached.extracted_comments {
              all_extracted_comments
                .lock()
                .expect("all_extract_comments lock failed")
                .insert(
                  filename.to_string(),
                  ExtractedCommentsInfo {
                    source: ec.source.clone(),
                    comments_file_name: ec.comments_file_name.clone(),
                  },
                );
            }
            return Ok(());
          }

          if let Some(counter) = &minimize_cache_counter {
            counter.miss();
          }
          Some(key)
        } else {
          None
        };
        let object_pool = tls.get_or(ObjectPool::default);
        let input_source_map = original_source.map(object_pool, &MapOptions::default());

        let js_minify_options = rspack_javascript_compiler::minify::JsMinifyOptions {
          minify: minimizer_options.minify.unwrap_or(true),
          compress: minimizer_options.compress.clone(),
          mangle: minimizer_options.mangle.clone(),
          format: minimizer_options.format.clone(),
          ecma: minimizer_options.ecma.clone(),
          source_map: BoolOrDataConfig::from_bool(input_source_map.is_some()),
          inline_sources_content: true, /* Using true so original_source can be None in SourceMapSource */
          module: is_module,
          ..Default::default()
        };
        let extract_comments_option = options.extract_comments.as_ref().map(|extract_comments| {
          let comments_filename = format!("{filename}.LICENSE.txt");
          let banner = match &extract_comments.banner {
            OptionWrapper::Default => {
              let dir = Path::new(filename).parent().expect("should has parent");
              let raw = Path::new(&comments_filename).strip_prefix(dir).expect("should has common prefix").to_string_lossy();
              let relative = raw.cow_replace('\\', "/");
              Some(format!("/*! LICENSE: {relative} */"))
            },
            OptionWrapper::Disabled => None,
            OptionWrapper::Custom(value) => Some(format!("/*! {value} */"))
          };
          NormalizedExtractComments {
            filename: comments_filename,
            condition: extract_comments_condition.as_ref().expect("must exist").clone(),
            banner
          }
        });

        let javascript_compiler = JavaScriptCompiler::new();
        let comments_op = |comments: &SingleThreadedComments| {
          if let Some(ref extract_comments) = extract_comments_option {
            let mut extracted_comments = vec![];
            // add all matched comments to source

            let (leading_trivial, trailing_trivial) = comments.borrow_all();

            leading_trivial.iter().for_each(|(_, comments)| {
              comments.iter().for_each(|c| {
                if extract_comments.condition.test(&c.text) {
                  let comment = match c.kind {
                    CommentKind::Line => {
                      format!("//{}", c.text)
                    }
                    CommentKind::Block => {
                      format!("/*{}*/", c.text)
                    }
                  };
                  if !extracted_comments.contains(&comment) {
                    extracted_comments.push(comment);
                  }
                }
              });
            });
            trailing_trivial.iter().for_each(|(_, comments)| {
              comments.iter().for_each(|c| {
                if extract_comments.condition.test(&c.text) {
                  let comment = match c.kind {
                    CommentKind::Line => {
                      format!("//{}", c.text)
                    }
                    CommentKind::Block => {
                      format!("/*{}*/", c.text)
                    }
                  };
                  if !extracted_comments.contains(&comment) {
                    extracted_comments.push(comment);
                  }
                }
              });
            });

            // if not matched comments, we don't need to emit .License.txt file
            if !extracted_comments.is_empty() {
              extracted_comments.sort();
              all_extracted_comments
                .lock()
                .expect("all_extract_comments lock failed")
                .insert(
                  filename.to_string(),
                  ExtractedCommentsInfo {
                    source: RawStringSource::from(extracted_comments.join("\n\n")).boxed(),
                    comments_file_name: extract_comments.filename.clone(),
                  },
                );
            }
          }
        };

        let mut output = match javascript_compiler.minify(
          swc_core::common::FileName::Custom(filename.to_string()),
          input,
          js_minify_options,
          Some(comments_op),
        ) {
            Ok(r) => r,
            Err(e) => {
              let errors = e.into_inner().into_iter().map(|err| {
                let mut d = Diagnostic::from(MinifyError(err));
                d.file = Some(filename.into());
                d
              }).collect::<Vec<_>>();
              tx.send(errors)?;
              return Ok(())
            },
        };

        let banner = if all_extracted_comments
          .lock()
          .expect("all_extract_comments lock failed")
          .contains_key(filename) {
            extract_comments_option.and_then(|option| option.banner)
          } else {
            None
          };

        let mut source = match banner {
            Some(banner) => {
              // There are two cases with banner:
              // 1. There's no shebang, we just prepend the banner to the code.
              // 2. There's a shebang, we prepend the shebang, then the banner, then the code.

              let mut shebang = None;
              if output.code.starts_with("#!") {
                if let Some((shebang_line, code)) = output.code.split_once('\n') {
                  shebang = Some(format!("{shebang_line}\n"));
                  output.code = code.to_string();
                } else {
                  // Handle shebang without newline - treat entire content as shebang
                  shebang = Some(output.code.clone());
                  output.code = String::new();
                }
              }

              let source = if let Some(source_map) = output.map {
                SourceMapSource::new(SourceMapSourceOptions {
                  value: output.code,
                  name: filename,
                  source_map,
                  original_source: None,
                  inner_source_map: input_source_map,
                  remove_original_source: true,
                })
                .boxed()
              } else {
                RawStringSource::from(output.code).boxed()
              };

              if let Some(shebang) = shebang {
                ConcatSource::new([
                  RawStringSource::from(shebang).boxed(),
                  RawStringSource::from(banner).boxed(),
                  RawStringSource::from_static("\n").boxed(),
                  source
                ]).boxed()
              } else {
                ConcatSource::new([
                  RawStringSource::from(banner).boxed(),
                  RawStringSource::from_static("\n").boxed(),
                  source
                ]).boxed()
              }
            },
            None => {
              // If there's no banner, we don't need to handle `output.code` at all.
              if let Some(source_map) = output.map {
                SourceMapSource::new(SourceMapSourceOptions {
                  value: output.code,
                  name: filename,
                  source_map,
                  original_source: None,
                  inner_source_map: input_source_map,
                  remove_original_source: true,
                })
                .boxed()
              } else {
                RawStringSource::from(output.code).boxed()
              }
            },
        };

        let mut updated_real_content_hash_record = None;
        if let Some(record) = record {
          let (restored_source, updated_record) =
            restore_real_content_hash_markers(source, record, &real_content_hash_markers)?;
          source = restored_source;
          updated_real_content_hash_record = updated_record;
        }

        // Store result in persistent cache (only when enabled)
        if let Some(cache_key) = cache_key {
          let extracted_comments_for_cache = all_extracted_comments
            .lock()
            .expect("all_extract_comments lock failed")
            .get(filename)
            .map(|ec| CachedExtractedComments {
              source: ec.source.clone(),
              comments_file_name: ec.comments_file_name.clone(),
            });

          new_persistent_cache_entries
            .lock()
            .expect("new_cache_entries lock failed")
            .push((
              cache_key,
              CachedMinimizeEntry {
                source: source.clone(),
                extracted_comments: extracted_comments_for_cache,
              },
            ));
        }

        original.set_source(Some(source));
        original.get_info_mut().minimized.replace(true);
        if let Some(record) = updated_real_content_hash_record {
          updated_real_content_hash_records
            .lock()
            .expect("updated_real_content_hash_records lock failed")
            .insert(filename.to_string(), record);
        }
      }

      Ok(())
  })?;

  // Restore persistent cache with new entries (only when enabled)
  if let Some(mut cache) = minimize_persistent_cache {
    for (key, entry) in new_persistent_cache_entries
      .into_inner()
      .expect("new_persistent_cache_entries lock failed")
    {
      cache.insert(key, entry);
    }
    compilation.minimize_persistent_cache_artifact = Some(cache);

    if let Some(counter) = minimize_cache_counter {
      logger.cache_end(counter);
    }
  }

  compilation.extend_diagnostics(rx.into_iter().flatten().collect::<Vec<_>>());
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

  // write all extracted comments to assets
  all_extracted_comments
    .lock()
    .expect("all_extracted_comments lock failed")
    .clone()
    .into_iter()
    .for_each(|(_, comments)| {
      compilation.emit_asset(
        comments.comments_file_name,
        CompilationAsset::new(
          Some(comments.source),
          AssetInfo {
            minimized: Some(true),
            ..Default::default()
          },
        ),
      )
    });

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
        "InvalidRealContentHashReplacementCoverage: asset '{}' has a {:?} content hash replacement for '{}' without a source range before minimization",
        filename,
        replacement.kind,
        replacement.old_hash
      ));
    };
    let (Ok(start), Ok(end)) = (usize::try_from(range.start), usize::try_from(range.end)) else {
      return Err(rspack_error::error!(
        "InvalidRealContentHashReplacementCoverage: asset '{}' has an invalid {:?} content hash replacement range for '{}' before minimization",
        filename,
        replacement.kind,
        replacement.old_hash
      ));
    };
    if input.get(start..end) != Some(replacement.old_hash.as_str()) {
      return Err(rspack_error::error!(
        "InvalidRealContentHashReplacementCoverage: asset '{}' has a stale {:?} content hash replacement range for '{}' before minimization",
        filename,
        replacement.kind,
        replacement.old_hash
      ));
    }
    let Some(marker) =
      real_content_hash_marker(&input, replacement_index, start..end, &replacement.old_hash)
    else {
      return Ok((input, Vec::new()));
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
  for marker in markers {
    let ranges = find_marker_ranges(&content, marker.marker.as_bytes());
    if ranges.len() != 1 {
      return Err(rspack_error::error!(
        "InvalidRealContentHashReplacementCoverage: expected exactly one preserved minimizer marker for content hash '{}' but found {}",
        marker.old_hash,
        ranges.len()
      ));
    }
    for range in ranges {
      marker_ranges.push((range, marker));
    }
  }
  marker_ranges.sort_unstable_by_key(|(range, _)| (range.start, range.end));

  let mut replace_source = ReplaceSource::new(source);
  let mut updated = record.clone();
  let mut removed_bytes = 0usize;
  for (range, marker) in marker_ranges {
    replace_source.replace(
      u32::try_from(range.start).expect("marker range start should fit in u32"),
      u32::try_from(range.end).expect("marker range end should fit in u32"),
      marker.old_hash.clone(),
      None,
    );
    let final_start = range.start - removed_bytes;
    let final_end = final_start + marker.old_hash.len();
    if let Some(replacement) = updated.replacements.get_mut(marker.replacement_index) {
      replacement.range = Some(
        u32::try_from(final_start).expect("replacement range start should fit in u32")
          ..u32::try_from(final_end).expect("replacement range end should fit in u32"),
      );
    }
    removed_bytes += marker.marker.len() - marker.old_hash.len();
  }

  Ok((replace_source.boxed(), Some(updated)))
}

fn real_content_hash_marker(
  input: &str,
  index: usize,
  range: Range<usize>,
  old_hash: &str,
) -> Option<String> {
  for salt in 0..32 {
    let mut hasher = FxHasher::default();
    input.hash(&mut hasher);
    index.hash(&mut hasher);
    range.start.hash(&mut hasher);
    range.end.hash(&mut hasher);
    old_hash.hash(&mut hasher);
    salt.hash(&mut hasher);

    let mut marker = format!(
      "__RSPACK_REAL_CONTENT_HASH_MARKER_{index}_{:016x}_{salt}__",
      hasher.finish()
    );
    while marker.len() <= old_hash.len() {
      marker.push_str("RCHMARKER");
    }
    if !input.contains(&marker) {
      return Some(marker);
    }
  }
  None
}

#[cfg(test)]
fn deterministic_real_content_hash_marker(index: usize, old_hash_len: usize) -> String {
  let mut marker = format!("__RSPACK_REAL_CONTENT_HASH_MARKER_{index}__");
  while marker.len() <= old_hash_len {
    marker.push_str("RCHMARKER");
  }
  marker
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
  use cow_utils::CowUtils;
  use rspack_core::{
    AssetHashRecord, ContentHashReplacement, ContentHashReplacementKind,
    rspack_sources::{RawStringSource, Source, SourceExt},
  };

  use super::{
    deterministic_real_content_hash_marker, mark_real_content_hash_replacements,
    restore_real_content_hash_markers,
  };

  #[test]
  fn real_content_hash_markers_preserve_duplicate_hash_ranges() {
    let input = "var a='hhhh';var b='hhhh';".to_string();
    let mut record = AssetHashRecord::default();
    record.replacements.push(ContentHashReplacement {
      old_hash: "hhhh".to_string(),
      range: Some(7..11),
      kind: ContentHashReplacementKind::Source,
    });
    record.replacements.push(ContentHashReplacement {
      old_hash: "hhhh".to_string(),
      range: Some(20..24),
      kind: ContentHashReplacementKind::Source,
    });

    let (marked, markers) =
      mark_real_content_hash_replacements(Some(&record), input.clone(), "asset.js")
        .expect("valid source range should markerize");
    assert_eq!(markers.len(), 2);
    assert_ne!(marked, input);

    let (restored, updated_record) =
      restore_real_content_hash_markers(RawStringSource::from(marked).boxed(), &record, &markers)
        .expect("markers should restore");

    assert_eq!(restored.source().into_string_lossy(), input);
    let updated_record = updated_record.expect("markers should update replacement ranges");
    assert_eq!(updated_record.replacements[0].range, Some(7..11));
    assert_eq!(updated_record.replacements[1].range, Some(20..24));
  }

  #[test]
  fn real_content_hash_markers_handle_hashes_longer_than_base_marker() {
    let old_hash = "a".repeat(64);
    let input = format!("var h='{old_hash}';");
    let mut record = AssetHashRecord::default();
    record.replacements.push(ContentHashReplacement {
      old_hash: old_hash.clone(),
      range: Some(7..71),
      kind: ContentHashReplacementKind::Source,
    });

    let (marked, markers) =
      mark_real_content_hash_replacements(Some(&record), input.clone(), "asset.js")
        .expect("valid source range should markerize");
    assert_eq!(markers.len(), 1);
    assert!(markers[0].marker.len() > old_hash.len());

    let (restored, updated_record) =
      restore_real_content_hash_markers(RawStringSource::from(marked).boxed(), &record, &markers)
        .expect("markers should restore");

    assert_eq!(restored.source().into_string_lossy(), input);
    let updated_record = updated_record.expect("marker should update replacement range");
    assert_eq!(updated_record.replacements[0].range, Some(7..71));
  }

  #[test]
  fn real_content_hash_markers_ignore_user_marker_like_strings() {
    let old_hash = "hhhh";
    let user_marker = deterministic_real_content_hash_marker(0, old_hash.len());
    let prefix = format!("var user='{user_marker}';var h='");
    let input = format!("{prefix}{old_hash}';");
    let range_start = u32::try_from(prefix.len()).expect("range start should fit");
    let range_end = range_start + u32::try_from(old_hash.len()).expect("hash len should fit");
    let mut record = AssetHashRecord::default();
    record.replacements.push(ContentHashReplacement {
      old_hash: old_hash.to_string(),
      range: Some(range_start..range_end),
      kind: ContentHashReplacementKind::Source,
    });

    let (marked, markers) =
      mark_real_content_hash_replacements(Some(&record), input.clone(), "asset.js")
        .expect("valid source range should markerize");
    assert_eq!(markers.len(), 1);

    let (restored, updated_record) =
      restore_real_content_hash_markers(RawStringSource::from(marked).boxed(), &record, &markers)
        .expect("markers should restore");

    assert_eq!(restored.source().into_string_lossy(), input);
    assert!(updated_record.is_some());
  }

  #[test]
  fn real_content_hash_marker_restore_rejects_missing_markers() {
    let input = "var a='aaaa';var b='bbbb';".to_string();
    let mut record = AssetHashRecord::default();
    record.replacements.push(ContentHashReplacement {
      old_hash: "aaaa".to_string(),
      range: Some(7..11),
      kind: ContentHashReplacementKind::Source,
    });
    record.replacements.push(ContentHashReplacement {
      old_hash: "bbbb".to_string(),
      range: Some(20..24),
      kind: ContentHashReplacementKind::Source,
    });

    let (mut marked, markers) =
      mark_real_content_hash_replacements(Some(&record), input, "asset.js")
        .expect("valid source range should markerize");
    assert_eq!(markers.len(), 2);
    marked = marked
      .cow_replace(&markers[0].marker, &markers[0].old_hash)
      .into_owned();

    let err =
      restore_real_content_hash_markers(RawStringSource::from(marked).boxed(), &record, &markers)
        .expect_err("missing marker should fail");

    assert!(
      err
        .to_string()
        .contains("InvalidRealContentHashReplacementCoverage")
    );
  }

  #[test]
  fn real_content_hash_markerize_rejects_stale_source_ranges() {
    let mut record = AssetHashRecord::default();
    record.replacements.push(ContentHashReplacement {
      old_hash: "hhhh".to_string(),
      range: Some(0..4),
      kind: ContentHashReplacementKind::Source,
    });

    let err =
      mark_real_content_hash_replacements(Some(&record), "var h='hhhh';".to_string(), "asset.js")
        .expect_err("stale source range should fail before minimization");

    assert!(
      err
        .to_string()
        .contains("InvalidRealContentHashReplacementCoverage")
    );
  }
}

pub fn match_object(obj: &PluginOptions, str: &str) -> bool {
  if let Some(condition) = &obj.test {
    if !condition.try_match(str) {
      return false;
    }
  } else if !JAVASCRIPT_ASSET_REGEXP.is_match(str) {
    return false;
  }
  if let Some(condition) = &obj.include
    && !condition.try_match(str)
  {
    return false;
  }
  if let Some(condition) = &obj.exclude
    && condition.try_match(str)
  {
    return false;
  }

  true
}

impl Plugin for SwcJsMinimizerRspackPlugin {
  fn name(&self) -> &'static str {
    PLUGIN_NAME
  }

  fn apply(&self, ctx: &mut rspack_core::ApplyContext<'_>) -> Result<()> {
    ctx.compiler_hooks.compilation.tap(compilation::new(self));
    ctx
      .compilation_hooks
      .process_assets
      .tap(process_assets::new(self));
    Ok(())
  }
}
