# Real content hash artifact design

## Context

Rspack's `RealContentHashPlugin` currently discovers content hash dependencies by scanning emitted asset source text. It builds an Aho-Corasick matcher from `asset.info.content_hash`, converts each source to text when possible, scans for old hash strings, derives a dependency graph, recomputes hashes in topological order, and finally rewrites assets with updated hash values.

That scan is expensive for large builds. It can force source materialization through `source()` / string conversion, and it rebuilds full `RawStringSource` values after replacement. The dependency information is also not truly discovered from user code: content hashes that need real-content-hash replacement are inserted by Rspack code generation or plugins. The compiler can record the relationship at insertion time instead of rediscovering it later.

The goal is to remove source scanning from real content hash processing. All content hash ownership, references, and replacement locations should be recorded explicitly during asset generation.

## Goals

- Store all real content hash records in a dedicated compilation artifact.
- Do not add real content hash tracking fields to `AssetInfo`.
- Avoid scanning generated source code for hash strings.
- Preserve topological real content hash recomputation.
- Use range-based `ReplaceSource` updates when a hash appears in source content.
- Support all assets that participate in content hash replacement, including JS, CSS, runtime modules, asset modules, copy/html/wasm-related assets, SRI, and third-party plugin assets.
- Fail clearly in strict mode when a content hash insertion was not recorded.

## Non-Goals

- Do not redesign the public meaning of `AssetInfo.content_hash`.
- Do not infer hash references from filenames or source text.
- Do not preserve compatibility by falling back to the old source scan.
- Do not introduce a `TrackedSource` wrapper as the primary metadata carrier.

## Data model

Add a compilation-owned artifact, for example `RealContentHashArtifact`, as the single source of truth for real content hash processing.

```rust
pub struct RealContentHashArtifact {
  pub asset_records: FxHashMap<String, AssetHashRecord>,
  pub hash_to_assets: FxHashMap<String, Vec<String>>,
}

pub struct AssetHashRecord {
  pub own_hashes: FxHashSet<String>,
  pub references: Vec<ContentHashReference>,
  pub replacements: Vec<ContentHashReplacement>,
}

pub struct ContentHashReference {
  pub referenced_hash: String,
  pub owner_hash: Option<String>,
  pub referenced_chunk: Option<ChunkUkey>,
  pub referenced_source_type: Option<SourceType>,
  pub kind: ContentHashReferenceKind,
}

pub struct ContentHashReplacement {
  pub old_hash: String,
  pub range: Option<std::ops::Range<u32>>,
  pub kind: ContentHashReplacementKind,
}
```

`own_hashes` records the hashes owned by an emitted asset. This mirrors the hashes that are also exposed through `AssetInfo.content_hash`, but the artifact is the internal dependency source.

`references` records hashes that this asset depends on. These edges replace the current scan-derived `referenced_hashes`.

`replacements` records where old hash values must be rewritten after new real content hashes are computed. Source replacements should carry byte ranges. Filename replacements do not need ranges and are applied through asset rename.

## Recording API

Expose explicit recording methods on the artifact and thin helpers on `Compilation`.

```rust
impl RealContentHashArtifact {
  pub fn record_asset_hashes(
    &mut self,
    asset: impl Into<String>,
    own_hashes: impl IntoIterator<Item = String>,
  );

  pub fn record_reference(
    &mut self,
    asset: &str,
    referenced_hash: &str,
    owner_hash: Option<&str>,
    meta: ContentHashReferenceMeta,
  );

  pub fn record_replacement(
    &mut self,
    asset: &str,
    old_hash: &str,
    range: Option<std::ops::Range<u32>>,
    kind: ContentHashReplacementKind,
  );
}
```

The API describes facts only. It must not compute hashes, mutate sources, or rename assets.

All content hash insertions must call the recording API explicitly:

- Filename insertions record the emitted asset name, the owned hash, and a filename replacement.
- Source insertions record the current asset, referenced hash, dependency metadata, and replacement range.
- Custom plugin assets record both owned hashes and any hash references they write into filenames or source content.

`get_path` and `get_path_with_info` should not implicitly record real content hash data. Callers are responsible for explicit recording after they know the asset being generated.

## Artifact lifecycle

The artifact lives on `Compilation`.

- `emit_asset` creates or merges the asset's record.
- `update_asset` preserves the record unless the updater explicitly replaces it.
- `rename_asset` moves the record from the old name to the new name.
- `delete_asset` removes the record.
- The artifact is discarded with the compilation.

This keeps `AssetInfo` unchanged and prevents real content hash tracking from leaking into public asset metadata.

## RealContentHashPlugin flow

The plugin should switch from scanning assets to reading `RealContentHashArtifact`.

1. Validate records.
   - Every sourced asset with `AssetInfo.content_hash` must have an artifact record.
   - Artifact `own_hashes` must cover `AssetInfo.content_hash`.
   - Every referenced hash must have an owner in `hash_to_assets`.
   - Source replacements must have ranges unless a plugin-specific updater can handle them.

2. Build the dependency graph.
   - Nodes are old hash strings.
   - For every asset record, each `own_hash` depends on each recorded `referenced_hash`.
   - Assets without own hashes can still have final replacements, but they do not create recomputation nodes.

3. Recompute hashes in topological batches.
   - Keep the existing batching model.
   - For each old hash, collect owning assets from `hash_to_assets`.
   - Create temporary source views by applying already-computed referenced hash replacements.
   - Preserve the current `without_own` behavior by replacing occurrences of the hash currently being computed with an empty string in its own assets.
   - Continue to call `RealContentHashPluginUpdateHash` before falling back to the default output hash algorithm.

4. Apply final updates.
   - Source replacements use `ReplaceSource::new(old_source)` and the recorded ranges.
   - Filename replacements compute the new name and use asset rename.
   - Asset info content hashes are updated from old hash to new hash as today.

The final stage should avoid rebuilding full `RawStringSource` values when recorded ranges are available.

## Migration points

Initial migration should cover every path that can put content hashes into final output:

- JS chunk assets from `JsPlugin`.
- CSS chunk assets from `CssPlugin` and `PluginCssExtract`.
- Wasm and asset module render manifest entries.
- Runtime modules that render chunk filename maps, main filename helpers, update filename helpers, and auto public path logic.
- Library/runtime helpers that embed chunk content hash values.
- HTML, copy, source map, and wasm-related plugins when they emit assets with content hashes.
- Subresource Integrity, while preserving its `RealContentHashPluginUpdateHash` hook.
- Binding/API surfaces needed by third-party plugins to record references explicitly.

When source content is assembled from strings, add a small helper or builder that records the current offset before appending a hash string. This is the preferred way to produce accurate ranges.

## Error handling

Strict mode should fail clearly instead of silently falling back to source scanning.

Recommended diagnostics:

- `MissingAssetRecord`: an asset has content hashes but no artifact record.
- `MissingOwnHash`: `AssetInfo.content_hash` and artifact `own_hashes` disagree.
- `MissingReferenceOwner`: a recorded reference points at a hash with no owner.
- `MissingReplacementRange`: a source replacement lacks a range and no updater covers it.
- `StaleReplacementRange`: the recorded range no longer contains the recorded old hash.
- `CircularHashDependency`: preserve the existing cycle detection behavior.

Debug builds may add assertions for stale ranges and impossible graph states, but normal builds should report actionable diagnostics.

## Testing

Keep all existing real content hash test cases passing.

Add focused tests for:

- Runtime chunk references to async chunk content hashes, proving topological order still works.
- Filename-only content hash replacement, proving rename works without touching source.
- Source range replacement, proving `ReplaceSource` is used and the final asset contains the new hash.
- Strict missing-record failure with a clear diagnostic.
- SRI update hash integration, proving the hook receives the already-updated source view.
- A plugin-generated asset with explicitly recorded references.

## Open implementation notes

- The artifact should likely live near existing compilation artifacts, not in `rspack_plugin_real_content_hash`.
- The real content hash plugin can initially keep much of the current topological batching structure while replacing `AssetData::new` and Aho-Corasick scanning with artifact reads.
- Range replacement should validate that the old hash is still present at the range before applying the replacement.
- Third-party binding APIs should be added only for recording, not for mutating the artifact internals directly.
