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
    if old_name == new_name {
      return;
    }

    let Some(mut record) = self.asset_records.remove(old_name) else {
      return;
    };

    let new_name_owned = new_name.to_string();
    for hash in &record.own_hashes {
      if let Some(assets) = self.hash_to_assets.get_mut(hash) {
        let mut has_new_name = false;
        assets.retain(|asset| {
          if asset == old_name {
            return false;
          }
          if asset == &new_name_owned {
            if has_new_name {
              return false;
            }
            has_new_name = true;
          }
          true
        });
        if !has_new_name {
          assets.push(new_name_owned.clone());
        }
      }
    }

    if let Some(existing_record) = self.asset_records.get_mut(new_name) {
      existing_record.own_hashes.extend(record.own_hashes);
      existing_record.references.append(&mut record.references);
      existing_record.replacements.append(&mut record.replacements);
    } else {
      self.asset_records.insert(new_name.to_string(), record);
    }
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
  fn rename_missing_asset_keeps_existing_records_unchanged() {
    let mut artifact = RealContentHashArtifact::default();
    artifact.record_asset_hashes("existing.js", ["aaaa".to_string()]);

    artifact.rename_asset("missing.js", "new.js");

    assert!(artifact.asset_records.get("missing.js").is_none());
    assert!(artifact.asset_records.get("new.js").is_none());
    assert!(artifact.asset_records.get("existing.js").is_some());
    assert_eq!(
      artifact.hash_to_assets.get("aaaa").expect("hash owner"),
      &vec!["existing.js".to_string()]
    );
  }

  #[test]
  fn rename_into_existing_asset_merges_records_and_reverse_index() {
    let mut artifact = RealContentHashArtifact::default();
    artifact.record_asset_hashes("old.js", ["aaaa".to_string(), "shared".to_string()]);
    artifact.record_asset_hashes("new.js", ["bbbb".to_string(), "shared".to_string()]);
    artifact.record_reference(
      "old.js",
      "old-reference",
      Some("aaaa"),
      ContentHashReferenceMeta::default(),
    );
    artifact.record_reference(
      "new.js",
      "new-reference",
      Some("bbbb"),
      ContentHashReferenceMeta::default(),
    );
    artifact.record_replacement(
      "old.js",
      "old-replacement",
      None,
      ContentHashReplacementKind::Source,
    );
    artifact.record_replacement(
      "new.js",
      "new-replacement",
      None,
      ContentHashReplacementKind::Source,
    );

    artifact.rename_asset("old.js", "new.js");

    assert!(artifact.asset_records.get("old.js").is_none());
    let record = artifact
      .asset_records
      .get("new.js")
      .expect("merged record should exist");
    assert_eq!(record.own_hashes.len(), 3);
    assert!(record.own_hashes.contains("aaaa"));
    assert!(record.own_hashes.contains("bbbb"));
    assert!(record.own_hashes.contains("shared"));
    assert_eq!(record.references.len(), 2);
    assert_eq!(record.replacements.len(), 2);
    assert_eq!(
      artifact.hash_to_assets.get("aaaa").expect("old hash owner"),
      &vec!["new.js".to_string()]
    );
    assert_eq!(
      artifact.hash_to_assets.get("bbbb").expect("new hash owner"),
      &vec!["new.js".to_string()]
    );
    assert_eq!(
      artifact.hash_to_assets.get("shared").expect("shared hash owner"),
      &vec!["new.js".to_string()]
    );
  }

  #[test]
  fn delete_one_of_two_assets_sharing_hash_preserves_other_reverse_index() {
    let mut artifact = RealContentHashArtifact::default();
    artifact.record_asset_hashes("first.js", ["shared".to_string()]);
    artifact.record_asset_hashes("second.js", ["shared".to_string()]);

    artifact.delete_asset("first.js");

    assert!(artifact.asset_records.get("first.js").is_none());
    assert!(artifact.asset_records.get("second.js").is_some());
    assert_eq!(
      artifact.hash_to_assets.get("shared").expect("shared hash owner"),
      &vec!["second.js".to_string()]
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
