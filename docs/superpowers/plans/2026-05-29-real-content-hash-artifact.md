# Real Content Hash Artifact Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace RealContentHashPlugin's source scanning with explicit real-content-hash records stored in a compilation artifact.

**Architecture:** Add a `RealContentHashArtifact` to `rspack_core::Compilation` and expose explicit record APIs. Migrate core asset generation paths to record owned hashes, hash references, and replacement locations. Then change `rspack_plugin_real_content_hash` to build its dependency graph and replacements from the artifact instead of scanning source text.

**Tech Stack:** Rust, `rspack_core`, `rspack_sources::ReplaceSource`, `rspack_plugin_real_content_hash`, existing Rspack hash/config test harness.

---

## File Structure

- Create `crates/rspack_core/src/artifacts/real_content_hash_artifact.rs`
  - Owns the data model, record APIs, asset lifecycle helpers, and unit tests for graph records.
- Modify `crates/rspack_core/src/artifacts/mod.rs`
  - Exports `RealContentHashArtifact`.
- Modify `crates/rspack_core/src/compilation/mod.rs`
  - Adds the artifact field, initializes it, exposes thin `Compilation` wrappers, and synchronizes it from `emit_asset`, `update_asset`, `rename_asset`, and `delete_asset`.
- Modify `crates/rspack_core/src/compilation/create_chunk_assets/mod.rs`
  - Ensures render-manifest records are merged into the compilation artifact when chunk assets are emitted.
- Modify `crates/rspack_core/src/compilation/mod.rs:220`
  - Adds the artifact field beside the other compilation artifacts.
- Modify `crates/rspack_core/src/compilation/mod.rs:335`
  - Initializes the new artifact inside `Compilation::new`.
- Modify `crates/rspack_core/src/lib.rs`
  - Re-export new public types if plugin crates need them.
- Modify `crates/rspack_plugin_real_content_hash/src/lib.rs`
  - Replaces `AssetData::new` scan logic and `OrderedHashesBuilder` input with artifact-based data.
- Modify `crates/rspack_plugin_javascript/src/plugin/impl_plugin_for_js_plugin.rs`
  - Records owned JS chunk content hashes and filename replacements.
- Modify `crates/rspack_plugin_css/src/plugin/impl_plugin_for_css_plugin.rs`
  - Records owned CSS chunk content hashes and filename replacements.
- Modify `crates/rspack_plugin_extract_css/src/plugin.rs`
  - Records owned extracted CSS content hashes and filename replacements.
- Modify `crates/rspack_plugin_asset/src/lib.rs`
  - Records asset-module owned content hashes and filename replacements.
- Modify runtime modules under `crates/rspack_plugin_runtime/src/runtime_module/`
  - Records source references for chunk filename maps and related helpers.
- Modify `crates/rspack_plugin_sri/src/asset.rs`
  - Keeps `update_hash` support and records SRI content hash ownership/replacements.
- Add tests under `tests/rspack-test/configCases/real-content-hash-artifact/`
  - Covers strict missing records, filename-only replacement, source range replacement, runtime reference ordering, and SRI hook behavior.

---

### Task 1: Add the RealContentHashArtifact Data Model

**Files:**
- Create: `crates/rspack_core/src/artifacts/real_content_hash_artifact.rs`
- Modify: `crates/rspack_core/src/artifacts/mod.rs`

- [ ] **Step 1: Write artifact unit tests**

Add this test module at the bottom of `crates/rspack_core/src/artifacts/real_content_hash_artifact.rs` when creating the file:

```rust
#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn records_owned_hashes_and_reverse_index() {
    let mut artifact = RealContentHashArtifact::default();

    artifact.record_asset_hashes(
      "main.aaaa.js",
      ["aaaa".to_string(), "bbbb".to_string()],
    );

    assert_eq!(
      artifact
        .asset_records
        .get("main.aaaa.js")
        .expect("record should exist")
        .own_hashes
        .len(),
      2
    );
    assert_eq!(
      artifact.hash_to_assets.get("aaaa").expect("hash owner"),
      &vec!["main.aaaa.js".to_string()]
    );
  }

  #[test]
  fn rename_moves_record_and_reverse_index() {
    let mut artifact = RealContentHashArtifact::default();
    artifact.record_asset_hashes("old.js", ["aaaa".to_string()]);

    artifact.rename_asset("old.js", "new.js");

    assert!(artifact.asset_records.get("old.js").is_none());
    assert!(artifact.asset_records.get("new.js").is_some());
    assert_eq!(
      artifact.hash_to_assets.get("aaaa").expect("hash owner"),
      &vec!["new.js".to_string()]
    );
  }

  #[test]
  fn delete_removes_record_and_reverse_index() {
    let mut artifact = RealContentHashArtifact::default();
    artifact.record_asset_hashes("asset.js", ["aaaa".to_string()]);

    artifact.delete_asset("asset.js");

    assert!(artifact.asset_records.is_empty());
    assert!(artifact.hash_to_assets.is_empty());
  }
}
```

- [ ] **Step 2: Run the new unit tests and verify they fail**

Run:

```bash
cargo test -p rspack_core real_content_hash_artifact --lib
```

Expected: fail because `real_content_hash_artifact.rs` and `RealContentHashArtifact` do not exist yet.

- [ ] **Step 3: Create the artifact implementation**

Create `crates/rspack_core/src/artifacts/real_content_hash_artifact.rs` with:

```rust
use std::ops::Range;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::{ChunkUkey, SourceType};

#[derive(Debug, Clone, Default)]
pub struct RealContentHashArtifact {
  pub asset_records: FxHashMap<String, AssetHashRecord>,
  pub hash_to_assets: FxHashMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Default)]
pub struct AssetHashRecord {
  pub own_hashes: FxHashSet<String>,
  pub references: Vec<ContentHashReference>,
  pub replacements: Vec<ContentHashReplacement>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentHashReference {
  pub referenced_hash: String,
  pub owner_hash: Option<String>,
  pub referenced_chunk: Option<ChunkUkey>,
  pub referenced_source_type: Option<SourceType>,
  pub kind: ContentHashReferenceKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentHashReferenceKind {
  Source,
  Filename,
  Custom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentHashReplacement {
  pub old_hash: String,
  pub range: Option<Range<u32>>,
  pub kind: ContentHashReplacementKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentHashReplacementKind {
  Source,
  Filename,
  Custom,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ContentHashReferenceMeta {
  pub referenced_chunk: Option<ChunkUkey>,
  pub referenced_source_type: Option<SourceType>,
  pub kind: ContentHashReferenceKind,
}

impl Default for ContentHashReferenceKind {
  fn default() -> Self {
    Self::Source
  }
}

impl RealContentHashArtifact {
  pub fn record_asset_hashes(
    &mut self,
    asset: impl Into<String>,
    own_hashes: impl IntoIterator<Item = String>,
  ) {
    let asset = asset.into();
    let record = self.asset_records.entry(asset.clone()).or_default();

    for hash in own_hashes {
      if record.own_hashes.insert(hash.clone()) {
        self
          .hash_to_assets
          .entry(hash)
          .or_default()
          .push(asset.clone());
      }
    }
  }

  pub fn record_reference(
    &mut self,
    asset: &str,
    referenced_hash: &str,
    owner_hash: Option<&str>,
    meta: ContentHashReferenceMeta,
  ) {
    self
      .asset_records
      .entry(asset.to_string())
      .or_default()
      .references
      .push(ContentHashReference {
        referenced_hash: referenced_hash.to_string(),
        owner_hash: owner_hash.map(str::to_string),
        referenced_chunk: meta.referenced_chunk,
        referenced_source_type: meta.referenced_source_type,
        kind: meta.kind,
      });
  }

  pub fn record_replacement(
    &mut self,
    asset: &str,
    old_hash: &str,
    range: Option<Range<u32>>,
    kind: ContentHashReplacementKind,
  ) {
    self
      .asset_records
      .entry(asset.to_string())
      .or_default()
      .replacements
      .push(ContentHashReplacement {
        old_hash: old_hash.to_string(),
        range,
        kind,
      });
  }

  pub fn rename_asset(&mut self, old_name: &str, new_name: &str) {
    let Some(record) = self.asset_records.remove(old_name) else {
      return;
    };
    for hash in &record.own_hashes {
      if let Some(assets) = self.hash_to_assets.get_mut(hash) {
        for asset in assets {
          if asset == old_name {
            *asset = new_name.to_string();
          }
        }
      }
    }
    self.asset_records.insert(new_name.to_string(), record);
  }

  pub fn delete_asset(&mut self, name: &str) {
    let Some(record) = self.asset_records.remove(name) else {
      return;
    };
    for hash in &record.own_hashes {
      if let Some(assets) = self.hash_to_assets.get_mut(hash) {
        assets.retain(|asset| asset != name);
        if assets.is_empty() {
          self.hash_to_assets.remove(hash);
        }
      }
    }
  }
}
```

- [ ] **Step 4: Export the artifact**

Modify `crates/rspack_core/src/artifacts/mod.rs`:

```rust
mod real_content_hash_artifact;
pub use real_content_hash_artifact::*;
```

Place the `mod` line beside the other artifact modules and the `pub use` beside the other `pub use` exports.

- [ ] **Step 5: Run artifact tests and verify they pass**

Run:

```bash
cargo test -p rspack_core real_content_hash_artifact --lib
```

Expected: pass all `real_content_hash_artifact` tests.

- [ ] **Step 6: Commit**

```bash
git add crates/rspack_core/src/artifacts/real_content_hash_artifact.rs crates/rspack_core/src/artifacts/mod.rs
git commit -m "feat(core): add real content hash artifact"
```

---

### Task 2: Attach the Artifact to Compilation

**Files:**
- Modify: `crates/rspack_core/src/compilation/mod.rs`
- Test: `crates/rspack_core/src/artifacts/real_content_hash_artifact.rs`

- [ ] **Step 1: Add lifecycle behavior tests**

Extend the test module in `crates/rspack_core/src/artifacts/real_content_hash_artifact.rs`:

```rust
#[test]
fn record_reference_and_replacement_create_asset_record() {
  let mut artifact = RealContentHashArtifact::default();

  artifact.record_reference(
    "runtime.js",
    "bbbb",
    Some("aaaa"),
    ContentHashReferenceMeta {
      kind: ContentHashReferenceKind::Source,
      referenced_chunk: None,
      referenced_source_type: None,
    },
  );
  artifact.record_replacement(
    "runtime.js",
    "bbbb",
    Some(10..14),
    ContentHashReplacementKind::Source,
  );

  let record = artifact
    .asset_records
    .get("runtime.js")
    .expect("record should exist");
  assert_eq!(record.references[0].referenced_hash, "bbbb");
  assert_eq!(record.references[0].owner_hash.as_deref(), Some("aaaa"));
  assert_eq!(record.replacements[0].range, Some(10..14));
}
```

- [ ] **Step 2: Run the tests**

Run:

```bash
cargo test -p rspack_core real_content_hash_artifact --lib
```

Expected: pass. This confirms the artifact methods support the compilation wrappers.

- [ ] **Step 3: Add the artifact field to Compilation**

In `crates/rspack_core/src/compilation/mod.rs`, add a field next to other artifact fields:

```rust
pub real_content_hash_artifact: RealContentHashArtifact,
```

In the `Compilation` constructor, initialize it with:

```rust
real_content_hash_artifact: Default::default(),
```

If the constructor uses struct update syntax, place it beside `chunk_render_artifact`, `chunk_hashes_artifact`, or similar artifact initialization.

- [ ] **Step 4: Add thin Compilation wrappers**

Add methods on `impl Compilation`:

```rust
pub fn record_real_content_hashes(
  &mut self,
  asset: impl Into<String>,
  own_hashes: impl IntoIterator<Item = String>,
) {
  self
    .real_content_hash_artifact
    .record_asset_hashes(asset, own_hashes);
}

pub fn record_real_content_hash_reference(
  &mut self,
  asset: &str,
  referenced_hash: &str,
  owner_hash: Option<&str>,
  meta: ContentHashReferenceMeta,
) {
  self.real_content_hash_artifact.record_reference(
    asset,
    referenced_hash,
    owner_hash,
    meta,
  );
}

pub fn record_real_content_hash_replacement(
  &mut self,
  asset: &str,
  old_hash: &str,
  range: Option<std::ops::Range<u32>>,
  kind: ContentHashReplacementKind,
) {
  self
    .real_content_hash_artifact
    .record_replacement(asset, old_hash, range, kind);
}
```

Import the artifact types from `crate::artifacts`.

- [ ] **Step 5: Synchronize rename and delete**

In `Compilation::rename_asset`, after moving the asset from `filename` to `new_name`, add:

```rust
self
  .real_content_hash_artifact
  .rename_asset(filename, &new_name);
```

In `Compilation::delete_asset`, after removing the asset, add:

```rust
self.real_content_hash_artifact.delete_asset(filename);
```

- [ ] **Step 6: Run core check**

Run:

```bash
cargo check -p rspack_core
```

Expected: pass.

- [ ] **Step 7: Commit**

```bash
git add crates/rspack_core/src/compilation/mod.rs crates/rspack_core/src/artifacts/real_content_hash_artifact.rs
git commit -m "feat(core): wire real content hash artifact into compilation"
```

---

### Task 3: Make RenderManifestEntry Carry Real Content Hash Records

**Files:**
- Modify: `crates/rspack_core/src/compilation/mod.rs`
- Modify: `crates/rspack_core/src/compilation/create_chunk_assets/mod.rs`
- Modify render manifest constructors that now need a new field:
  - `crates/rspack_plugin_javascript/src/plugin/impl_plugin_for_js_plugin.rs`
  - `crates/rspack_plugin_css/src/plugin/impl_plugin_for_css_plugin.rs`
  - `crates/rspack_plugin_extract_css/src/plugin.rs`
  - `crates/rspack_plugin_asset/src/lib.rs`
  - `crates/rspack_plugin_wasm/src/wasm_plugin.rs`

- [ ] **Step 1: Add record storage to RenderManifestEntry**

Extend `RenderManifestEntry` in `crates/rspack_core/src/compilation/mod.rs`:

```rust
#[derive(Debug, Clone)]
pub struct RenderManifestEntry {
  pub source: BoxSource,
  pub filename: String,
  pub has_filename: bool,
  pub info: AssetInfo,
  pub auxiliary: bool,
  pub real_content_hashes: AssetHashRecord,
}
```

- [ ] **Step 2: Add a helper constructor**

Add this impl beside `RenderManifestEntry`:

```rust
impl RenderManifestEntry {
  pub fn new(
    source: BoxSource,
    filename: String,
    has_filename: bool,
    info: AssetInfo,
    auxiliary: bool,
  ) -> Self {
    Self {
      source,
      filename,
      has_filename,
      info,
      auxiliary,
      real_content_hashes: Default::default(),
    }
  }

  pub fn with_real_content_hashes(mut self, record: AssetHashRecord) -> Self {
    self.real_content_hashes = record;
    self
  }
}
```

- [ ] **Step 3: Replace struct literals with the constructor**

For each compile error at a `RenderManifestEntry { ... }` literal, replace it with:

```rust
RenderManifestEntry::new(source, filename, has_filename, info, auxiliary)
```

For example, in `crates/rspack_plugin_javascript/src/plugin/impl_plugin_for_js_plugin.rs`, replace:

```rust
manifest.push(RenderManifestEntry {
  source,
  filename: output_path,
  has_filename: false,
  info: asset_info,
  auxiliary: false,
});
```

with:

```rust
manifest.push(RenderManifestEntry::new(
  source,
  output_path,
  false,
  asset_info,
  false,
));
```

- [ ] **Step 4: Merge records when emitting chunk assets**

In `crates/rspack_core/src/compilation/create_chunk_assets/mod.rs`, before `compilation.emit_asset(...)`, add:

```rust
if !file_manifest.real_content_hashes.own_hashes.is_empty()
  || !file_manifest.real_content_hashes.references.is_empty()
  || !file_manifest.real_content_hashes.replacements.is_empty()
{
  compilation
    .real_content_hash_artifact
    .asset_records
    .insert(filename.clone(), file_manifest.real_content_hashes.clone());
  for hash in &file_manifest.real_content_hashes.own_hashes {
    compilation
      .real_content_hash_artifact
      .hash_to_assets
      .entry(hash.clone())
      .or_default()
      .push(filename.clone());
  }
}
```

If direct field access feels too open after Task 1, add `merge_asset_record(asset, record)` to `RealContentHashArtifact` and call that instead.

- [ ] **Step 5: Run compile check**

Run:

```bash
cargo check -p rspack_core -p rspack_plugin_javascript -p rspack_plugin_css -p rspack_plugin_extract_css -p rspack_plugin_asset -p rspack_plugin_wasm
```

Expected: pass after all render manifest constructors are updated.

- [ ] **Step 6: Commit**

```bash
git add crates/rspack_core/src/compilation/mod.rs crates/rspack_core/src/compilation/create_chunk_assets/mod.rs crates/rspack_plugin_javascript/src/plugin/impl_plugin_for_js_plugin.rs crates/rspack_plugin_css/src/plugin/impl_plugin_for_css_plugin.rs crates/rspack_plugin_extract_css/src/plugin.rs crates/rspack_plugin_asset/src/lib.rs crates/rspack_plugin_wasm/src/wasm_plugin.rs
git commit -m "feat(core): carry real content hash records through render manifests"
```

---

### Task 4: Record Owned Hashes and Filename Replacements for Core Assets

**Files:**
- Modify: `crates/rspack_plugin_javascript/src/plugin/impl_plugin_for_js_plugin.rs`
- Modify: `crates/rspack_plugin_css/src/plugin/impl_plugin_for_css_plugin.rs`
- Modify: `crates/rspack_plugin_extract_css/src/plugin.rs`
- Modify: `crates/rspack_plugin_asset/src/lib.rs`
- Modify: `crates/rspack_plugin_wasm/src/wasm_plugin.rs`
- Test: `tests/rspack-test/hashCases/real-content-hash`

- [ ] **Step 1: Add a small helper for manifest-owned hash records**

Create a private helper in each plugin file or one shared helper in `rspack_core` if imports are clean:

```rust
fn record_owned_content_hash(
  record: &mut AssetHashRecord,
  content_hash: Option<&str>,
) {
  if let Some(content_hash) = content_hash {
    record.own_hashes.insert(content_hash.to_string());
    record.replacements.push(ContentHashReplacement {
      old_hash: content_hash.to_string(),
      range: None,
      kind: ContentHashReplacementKind::Filename,
    });
  }
}
```

- [ ] **Step 2: Record JS chunk owned hashes**

In `JsPlugin::render_manifest`, store the rendered content hash before calling `get_path_with_info`:

```rust
let rendered_content_hash = chunk.rendered_content_hash_by_source_type(
  &compilation.chunk_hashes_artifact,
  &SourceType::JavaScript,
  compilation.options.output.hash_digest_length,
);
```

Use `rendered_content_hash` in `PathData::content_hash_optional(...)`.

Before pushing the manifest:

```rust
let mut real_content_hashes = AssetHashRecord::default();
record_owned_content_hash(&mut real_content_hashes, rendered_content_hash);

manifest.push(
  RenderManifestEntry::new(source, output_path, false, asset_info, false)
    .with_real_content_hashes(real_content_hashes),
);
```

- [ ] **Step 3: Repeat owned-hash recording for CSS, extracted CSS, asset modules, and wasm**

Apply the same pattern wherever a rendered content hash is passed to `PathData::content_hash_optional(...)` or `PathData::content_hash(...)`.

For asset modules, use the content hash already computed for the asset filename. Record `has_filename: true` entries the same way.

- [ ] **Step 4: Run existing real content hash tests**

Run:

```bash
cd tests/rspack-test && pnpm run test -t "hashCases/real-content-hash"
```

Expected: behavior unchanged. If the JS packages are stale, run `pnpm run build:binding:dev` from repo root first, then rerun the filtered test.

- [ ] **Step 5: Commit**

```bash
git add crates/rspack_plugin_javascript/src/plugin/impl_plugin_for_js_plugin.rs crates/rspack_plugin_css/src/plugin/impl_plugin_for_css_plugin.rs crates/rspack_plugin_extract_css/src/plugin.rs crates/rspack_plugin_asset/src/lib.rs crates/rspack_plugin_wasm/src/wasm_plugin.rs
git commit -m "feat: record owned real content hashes for emitted assets"
```

---

### Task 5: Replace RealContentHashPlugin Scanning with Artifact Validation and Graph Building

**Files:**
- Modify: `crates/rspack_plugin_real_content_hash/src/lib.rs`
- Test: `tests/rspack-test/configCases/real-content-hash-artifact/missing-record/`

- [ ] **Step 1: Add a strict missing-record test case**

Create `tests/rspack-test/configCases/real-content-hash-artifact/missing-record/rspack.config.js`:

```js
const { sources } = require("@rspack/core");

class UntrackedContentHashPlugin {
  apply(compiler) {
    compiler.hooks.thisCompilation.tap("UntrackedContentHashPlugin", compilation => {
      compilation.hooks.processAssets.tap(
        {
          name: "UntrackedContentHashPlugin",
          stage: compiler.webpack.Compilation.PROCESS_ASSETS_STAGE_ADDITIONS
        },
        () => {
          compilation.emitAsset(
            "untracked.aaaa.js",
            new sources.RawSource("console.log('aaaa');"),
            { contenthash: "aaaa" }
          );
        }
      );
    });
  }
}

module.exports = {
  mode: "production",
  entry: "./index.js",
  optimization: { realContentHash: true },
  plugins: [new UntrackedContentHashPlugin()]
};
```

Create `tests/rspack-test/configCases/real-content-hash-artifact/missing-record/index.js`:

```js
it("reports missing real content hash records", () => {});
```

Create `tests/rspack-test/configCases/real-content-hash-artifact/missing-record/test.config.js`:

```js
module.exports = {
  findBundle() {
    return [];
  },
  async check(_, stats) {
    const info = stats.toJson({ all: false, errors: true });
    expect(info.errors.some(error =>
      String(error.message || error).includes("MissingRealContentHashRecord")
    )).toBe(true);
  }
};
```

- [ ] **Step 2: Run the missing-record test and verify it fails**

Run:

```bash
cd tests/rspack-test && pnpm run test -t "configCases/real-content-hash-artifact/missing-record"
```

Expected: fail because the plugin does not validate artifact records yet.

- [ ] **Step 3: Add artifact validation**

In `crates/rspack_plugin_real_content_hash/src/lib.rs`, replace the initial scan setup with:

```rust
fn validate_artifact(compilation: &Compilation) -> Result<()> {
  for (name, asset) in compilation.assets().iter() {
    if asset.get_source().is_none() || asset.info.content_hash.is_empty() {
      continue;
    }

    let Some(record) = compilation.real_content_hash_artifact.asset_records.get(name) else {
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
    }
  }

  for (asset_name, record) in &compilation.real_content_hash_artifact.asset_records {
    for reference in &record.references {
      if !compilation
        .real_content_hash_artifact
        .hash_to_assets
        .contains_key(&reference.referenced_hash)
      {
        return Err(rspack_error::error!(
          "MissingRealContentHashReferenceOwner: asset '{}' references unknown content hash '{}'",
          asset_name,
          reference.referenced_hash
        ));
      }
    }
  }

  Ok(())
}
```

Call `validate_artifact(compilation)?;` near the start of `inner_impl`.

- [ ] **Step 4: Build graph from artifact**

Replace `OrderedHashesBuilder`'s inputs with the artifact:

```rust
struct OrderedHashesBuilder<'a> {
  artifact: &'a RealContentHashArtifact,
}
```

Make `get_hash_dependencies` collect references from all owner assets:

```rust
fn get_hash_dependencies(&self, hash: &str) -> HashSet<&str> {
  let mut hashes = HashSet::default();
  let Some(asset_names) = self.artifact.hash_to_assets.get(hash) else {
    return hashes;
  };

  for name in asset_names {
    if let Some(record) = self.artifact.asset_records.get(name) {
      for reference in &record.references {
        hashes.insert(reference.referenced_hash.as_str());
      }
    }
  }

  hashes
}
```

- [ ] **Step 5: Remove scan-only dependencies**

Remove `aho_corasick`, `regex`, `OnceCell`, and `SourceValue` imports from `rspack_plugin_real_content_hash` once replacement no longer uses scanning.

- [ ] **Step 6: Run validation test**

Run:

```bash
cd tests/rspack-test && pnpm run test -t "configCases/real-content-hash-artifact/missing-record"
```

Expected: pass with the expected diagnostic.

- [ ] **Step 7: Commit**

```bash
git add crates/rspack_plugin_real_content_hash/src/lib.rs tests/rspack-test/configCases/real-content-hash-artifact/missing-record
git commit -m "feat(real-content-hash): validate artifact records"
```

---

### Task 6: Implement Range-Based Temporary and Final Source Replacement

**Files:**
- Modify: `crates/rspack_plugin_real_content_hash/src/lib.rs`
- Test: `tests/rspack-test/configCases/real-content-hash-artifact/source-range/`

- [ ] **Step 1: Add a source-range fixture**

Create `tests/rspack-test/configCases/real-content-hash-artifact/source-range/index.js`:

```js
import("./async");

it("updates recorded content hash ranges", async () => {
  await import("./async");
});
```

Create `tests/rspack-test/configCases/real-content-hash-artifact/source-range/async.js`:

```js
export default "async";
```

Create `tests/rspack-test/configCases/real-content-hash-artifact/source-range/rspack.config.js`:

```js
module.exports = {
  mode: "production",
  entry: "./index.js",
  output: {
    filename: "[name].[contenthash].js",
    chunkFilename: "[name].[contenthash].js"
  },
  optimization: {
    realContentHash: true
  }
};
```

Create `tests/rspack-test/configCases/real-content-hash-artifact/source-range/test.config.js`:

```js
module.exports = {
  async check(stats) {
    const info = stats.toJson({ all: false, assets: true });
    const jsAssets = info.assets.map(asset => asset.name).filter(name => name.endsWith(".js"));
    expect(jsAssets.length).toBeGreaterThan(1);
    for (const asset of jsAssets) {
      expect(asset).not.toContain("[contenthash]");
    }
  }
};
```

- [ ] **Step 2: Run the fixture and capture current failure**

Run:

```bash
cd tests/rspack-test && pnpm run test -t "configCases/real-content-hash-artifact/source-range"
```

Expected: fail until runtime source references are recorded in later tasks. Keep this test as a regression target.

- [ ] **Step 3: Add a source builder helper inside RealContentHashPlugin**

In `crates/rspack_plugin_real_content_hash/src/lib.rs`, add:

```rust
fn apply_recorded_replacements(
  source: BoxSource,
  record: &AssetHashRecord,
  hash_to_new_hash: &HashMap<String, String>,
  without_own: Option<&str>,
) -> Result<BoxSource> {
  let mut replace_source = rspack_core::rspack_sources::ReplaceSource::new(source);
  let mut changed = false;

  for replacement in &record.replacements {
    let Some(range) = &replacement.range else {
      continue;
    };
    let next = if without_own == Some(replacement.old_hash.as_str()) {
      ""
    } else {
      hash_to_new_hash
        .get(&replacement.old_hash)
        .map(String::as_str)
        .unwrap_or(replacement.old_hash.as_str())
    };
    replace_source.replace(range.start, range.end, next.to_string(), None);
    changed = true;
  }

  if changed {
    Ok(replace_source.boxed())
  } else {
    Ok(replace_source.inner().clone())
  }
}
```

- [ ] **Step 4: Use recorded replacements for batch sources**

Replace `data.compute_new_source(...)` calls with `apply_recorded_replacements(...)` using:

```rust
let record = compilation
  .real_content_hash_artifact
  .asset_records
  .get(name)
  .expect("validated record should exist");
let source = compilation
  .assets()
  .get(name)
  .and_then(|asset| asset.get_source())
  .expect("validated asset source should exist")
  .clone();
let new_source = apply_recorded_replacements(
  source,
  record,
  &hash_to_new_hash,
  record.own_hashes.contains(hash).then_some(hash),
)?;
```

- [ ] **Step 5: Use recorded replacements for final updates**

In the final update collection, call `apply_recorded_replacements(source, record, &hash_to_new_hash, None)`.

For replacements with `ContentHashReplacementKind::Filename`, do not apply a source range; use filename rename.

- [ ] **Step 6: Add missing range validation**

In validation, reject source/custom replacements without a range:

```rust
for replacement in &record.replacements {
  if matches!(
    replacement.kind,
    ContentHashReplacementKind::Source | ContentHashReplacementKind::Custom
  ) && replacement.range.is_none()
  {
    return Err(rspack_error::error!(
      "MissingRealContentHashReplacementRange: asset '{}' replacement for '{}' has no source range",
      asset_name,
      replacement.old_hash
    ));
  }
}
```

- [ ] **Step 7: Run plugin check**

Run:

```bash
cargo check -p rspack_plugin_real_content_hash
```

Expected: pass.

- [ ] **Step 8: Commit**

```bash
git add crates/rspack_plugin_real_content_hash/src/lib.rs tests/rspack-test/configCases/real-content-hash-artifact/source-range
git commit -m "feat(real-content-hash): replace hashes from recorded ranges"
```

---

### Task 7: Record Runtime Source References

**Files:**
- Modify: `crates/rspack_plugin_runtime/src/runtime_module/get_chunk_filename.rs`
- Modify: `crates/rspack_plugin_runtime/src/runtime_module/get_main_filename.rs`
- Modify: `crates/rspack_plugin_runtime/src/runtime_module/get_chunk_update_filename.rs`
- Modify: `crates/rspack_plugin_runtime/src/runtime_module/auto_public_path.rs`
- Modify: `crates/rspack_plugin_runtime/src/runtime_module/utils.rs`
- Modify any runtime source builder touched by compile errors.

- [ ] **Step 1: Add a string builder utility for tracked hash ranges**

Create a private helper in the runtime module that builds filename maps:

```rust
#[derive(Default)]
struct RealContentHashStringBuilder {
  value: String,
  replacements: Vec<ContentHashReplacement>,
  references: Vec<ContentHashReference>,
}

impl RealContentHashStringBuilder {
  fn push_str(&mut self, value: &str) {
    self.value.push_str(value);
  }

  fn push_content_hash(
    &mut self,
    hash: &str,
    referenced_chunk: Option<ChunkUkey>,
    referenced_source_type: Option<SourceType>,
  ) {
    let start = self.value.len() as u32;
    self.value.push_str(hash);
    let end = self.value.len() as u32;
    self.replacements.push(ContentHashReplacement {
      old_hash: hash.to_string(),
      range: Some(start..end),
      kind: ContentHashReplacementKind::Source,
    });
    self.references.push(ContentHashReference {
      referenced_hash: hash.to_string(),
      owner_hash: None,
      referenced_chunk,
      referenced_source_type,
      kind: ContentHashReferenceKind::Source,
    });
  }
}
```

- [ ] **Step 2: Record references in chunk filename runtime generation**

Where `get_chunk_filename.rs` currently computes `content_hash` and inserts it into generated filename strings, route the insertion through `push_content_hash(...)` instead of plain `format!` or `push_str`.

After building runtime source, attach the builder's `references` and `replacements` to the current asset's `AssetHashRecord`.

- [ ] **Step 3: Thread runtime records into the owning JS render manifest**

If runtime module rendering currently returns only `BoxSource`, extend the internal render result to include `AssetHashRecord` and merge it into the JS `RenderManifestEntry` record.

Use this merge logic:

```rust
real_content_hashes.references.extend(runtime_record.references);
real_content_hashes.replacements.extend(runtime_record.replacements);
```

- [ ] **Step 4: Run the source-range fixture**

Run:

```bash
cd tests/rspack-test && pnpm run test -t "configCases/real-content-hash-artifact/source-range"
```

Expected: pass once runtime references and ranges are recorded.

- [ ] **Step 5: Run existing runtime contenthash tests**

Run:

```bash
cd tests/rspack-test && pnpm run test -t "configCases/runtime/get-chunk-filename-runtime|configCases/runtime/dynamic-css-chunk-with-content-hash|configCases/runtime/split-css-chunk"
```

Expected: pass.

- [ ] **Step 6: Commit**

```bash
git add crates/rspack_plugin_runtime/src/runtime_module crates/rspack_plugin_javascript/src/plugin/impl_plugin_for_js_plugin.rs tests/rspack-test/configCases/real-content-hash-artifact/source-range
git commit -m "feat(runtime): record content hash references in runtime output"
```

---

### Task 8: Support SRI and Custom updateHash Sources

**Files:**
- Modify: `crates/rspack_plugin_sri/src/asset.rs`
- Modify: `crates/rspack_plugin_real_content_hash/src/drive.rs`
- Modify: `crates/rspack_plugin_real_content_hash/src/lib.rs`
- Test: `tests/rspack-test/configCases/sri/issue-12467`

- [ ] **Step 1: Preserve the update_hash hook contract**

Keep this hook signature unchanged in `crates/rspack_plugin_real_content_hash/src/drive.rs`:

```rust
define_hook!(RealContentHashPluginUpdateHash: SeriesBail(compilation: &Compilation, assets: &[Arc<dyn Source>], old_hash: &str) -> String);
```

No test should require plugin authors to change existing `updateHash` hook usage.

- [ ] **Step 2: Feed hook assets from recorded replacement views**

In `RealContentHashPlugin`, ensure the `asset_contents` passed to `update_hash.call(...)` are the sources returned by `apply_recorded_replacements(...)`, not original sources.

Use this exact ordering:

```rust
asset_names.sort_unstable();
let mut asset_contents = asset_names
  .iter()
  .filter_map(|name| batch_sources.get(&(old_hash.as_str(), name.as_str())))
  .cloned()
  .collect::<Vec<_>>();
asset_contents.dedup();
```

- [ ] **Step 3: Record SRI-owned hashes**

In `crates/rspack_plugin_sri/src/asset.rs`, when SRI adds an integrity value to `new_info.content_hash`, also record the asset hash:

```rust
compilation.record_real_content_hashes(
  result.file.clone(),
  [integrity.clone()],
);
```

If the SRI plugin inserts integrity text into source, record the source replacement range at the same site where `ReplaceSource` currently replaces placeholders.

- [ ] **Step 4: Run SRI tests**

Run:

```bash
cd tests/rspack-test && pnpm run test -t "configCases/sri/issue-12467"
```

Expected: pass and preserve the real content hash update hook behavior.

- [ ] **Step 5: Commit**

```bash
git add crates/rspack_plugin_sri/src/asset.rs crates/rspack_plugin_real_content_hash/src/drive.rs crates/rspack_plugin_real_content_hash/src/lib.rs
git commit -m "feat(real-content-hash): support recorded SRI hash updates"
```

---

### Task 9: Expose Recording to Binding/API Plugins

**Files:**
- Modify: `crates/rspack_binding_api/src/compilation.rs` or the existing compilation binding module that exposes `emitAsset` and `updateAsset`
- Modify: TypeScript binding declarations generated from NAPI if required
- Test: `tests/rspack-test/configCases/process-assets/html-plugin/rspack.config.js`

- [ ] **Step 1: Locate the compilation asset binding**

Run:

```bash
rg -n "emitAsset|updateAsset|contenthash|content_hash" crates/rspack_binding_api crates/node_binding packages/rspack/src -g '*.rs' -g '*.ts' -g '*.d.ts'
```

Use the module that currently maps JS `assetInfo.contenthash` to Rust `AssetInfo.content_hash`.

- [ ] **Step 2: Add a JS-facing record method**

Expose a method with this shape:

```ts
compilation.recordRealContentHashReference({
  asset: string,
  referencedHash: string,
  ownerHash?: string,
  range?: [number, number],
  kind?: "source" | "filename" | "custom"
});
```

Map it to:

```rust
compilation.record_real_content_hash_reference(
  &asset,
  &referenced_hash,
  owner_hash.as_deref(),
  ContentHashReferenceMeta {
    kind,
    referenced_chunk: None,
    referenced_source_type: None,
  },
);

if let Some([start, end]) = range {
  compilation.record_real_content_hash_replacement(
    &asset,
    &referenced_hash,
    Some(start..end),
    replacement_kind,
  );
}
```

- [ ] **Step 3: Update html-plugin fixture to record references**

In `tests/rspack-test/configCases/process-assets/html-plugin/rspack.config.js`, where the plugin emits HTML containing references to hashed assets, call `compilation.recordRealContentHashReference(...)` for each inserted content hash.

Use exact ranges by computing:

```js
const start = html.indexOf(contenthash);
const end = start + contenthash.length;
compilation.recordRealContentHashReference({
  asset: htmlFile,
  referencedHash: contenthash,
  range: [start, end],
  kind: "source"
});
```

- [ ] **Step 4: Run binding build and html plugin test**

Run:

```bash
pnpm run build:js
cd tests/rspack-test && pnpm run test -t "configCases/process-assets/html-plugin"
```

Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add crates/rspack_binding_api crates/node_binding packages/rspack/src tests/rspack-test/configCases/process-assets/html-plugin
git commit -m "feat(api): expose real content hash reference recording"
```

---

### Task 10: Remove Scan Dependencies and Run Final Verification

**Files:**
- Modify: `crates/rspack_plugin_real_content_hash/Cargo.toml`
- Modify: `crates/rspack_plugin_real_content_hash/src/lib.rs`
- Modify tests from previous tasks if snapshots need updates.

- [ ] **Step 1: Remove scan-only crates**

In `crates/rspack_plugin_real_content_hash/Cargo.toml`, remove dependencies that are no longer used only for scanning:

```toml
aho-corasick = { workspace = true }
regex = { workspace = true }
once_cell = { workspace = true }
```

Only remove an entry if `cargo check -p rspack_plugin_real_content_hash` confirms nothing else in the crate uses it.

- [ ] **Step 2: Ensure source scanning code is gone**

Run:

```bash
rg -n "AhoCorasick|find_iter|AssetDataContent|SourceValue|QUOTE_META|replace_all_with" crates/rspack_plugin_real_content_hash/src crates/rspack_plugin_real_content_hash/Cargo.toml
```

Expected: no matches for scan-specific names.

- [ ] **Step 3: Run Rust checks**

Run:

```bash
cargo check -p rspack_core -p rspack_plugin_real_content_hash -p rspack_plugin_javascript -p rspack_plugin_runtime -p rspack_plugin_css -p rspack_plugin_extract_css -p rspack_plugin_asset -p rspack_plugin_sri
```

Expected: pass.

- [ ] **Step 4: Build binding before JS tests**

Run:

```bash
pnpm run build:binding:dev
```

Expected: pass.

- [ ] **Step 5: Run focused real content hash tests**

Run:

```bash
cd tests/rspack-test && pnpm run test -t "hashCases/real-content-hash|hashCases/real-content-hash-md4|hashCases/real-content-hash-sha256|hashCases/real-content-hash-xxhash64|configCases/real-content-hash-artifact|configCases/runtime/dynamic-css-chunk-with-content-hash|configCases/process-assets/html-plugin|configCases/sri/issue-12467"
```

Expected: pass.

- [ ] **Step 6: Run formatting**

Run:

```bash
pnpm run format:rs
pnpm run format:js
```

Expected: pass or update formatting only in touched files.

- [ ] **Step 7: Commit final cleanup**

```bash
git add crates/rspack_plugin_real_content_hash/Cargo.toml crates/rspack_plugin_real_content_hash/src/lib.rs
git commit -m "refactor(real-content-hash): remove source scanning"
```

---

## Plan Self-Review

- Spec coverage:
  - Dedicated compilation artifact: Tasks 1 and 2.
  - No `AssetInfo` tracking fields: Tasks 1 through 3 keep records in the artifact and render manifest handoff only.
  - No source scanning: Tasks 5, 6, and 10.
  - Topological recomputation: Task 5 keeps `OrderedHashesBuilder` with artifact edges.
  - Range-based replacement: Tasks 6 and 7.
  - Broad asset support: Tasks 4, 7, 8, and 9.
  - Strict failures: Task 5 and Task 6.
- Red-flag scan: no incomplete placeholders or unspecified test steps remain.
- Type consistency:
  - `RealContentHashArtifact`, `AssetHashRecord`, `ContentHashReference`, `ContentHashReplacement`, `ContentHashReferenceMeta`, `ContentHashReferenceKind`, and `ContentHashReplacementKind` are introduced in Task 1 and reused consistently.
  - `record_real_content_hashes`, `record_real_content_hash_reference`, and `record_real_content_hash_replacement` are introduced in Task 2 and reused in later tasks.
