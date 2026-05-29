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

impl AssetHashRecord {
  pub fn is_empty(&self) -> bool {
    self.own_hashes.is_empty() && self.references.is_empty() && self.replacements.is_empty()
  }

  pub fn extend(&mut self, mut other: AssetHashRecord) {
    self.own_hashes.extend(other.own_hashes);
    self.references.append(&mut other.references);
    self.replacements.append(&mut other.replacements);
  }

  pub fn shift_source_ranges(&mut self, offset: u32) {
    if offset == 0 {
      return;
    }

    for replacement in &mut self.replacements {
      if !matches!(
        replacement.kind,
        ContentHashReplacementKind::Source | ContentHashReplacementKind::Custom
      ) {
        continue;
      }
      if let Some(range) = &mut replacement.range {
        range.start = range
          .start
          .checked_add(offset)
          .expect("content hash replacement range start should fit in u32");
        range.end = range
          .end
          .checked_add(offset)
          .expect("content hash replacement range end should fit in u32");
      }
    }
  }
}

pub fn record_manifest_owned_content_hash(
  record: &mut AssetHashRecord,
  content_hash: Option<&str>,
) {
  if let Some(content_hash) = content_hash {
    record.own_hashes.insert(content_hash.to_string());
  }
}

pub fn record_manifest_filename_content_hashes<'a>(
  record: &mut AssetHashRecord,
  content_hashes: impl IntoIterator<Item = &'a String>,
) {
  for content_hash in content_hashes {
    record.own_hashes.insert(content_hash.to_string());
    record.replacements.push(ContentHashReplacement {
      old_hash: content_hash.to_string(),
      range: None,
      kind: ContentHashReplacementKind::Filename,
    });
  }
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

  pub fn merge_asset_record(&mut self, asset: impl Into<String>, record: AssetHashRecord) {
    let asset = asset.into();
    self.record_asset_hashes(asset.clone(), record.own_hashes.iter().cloned());
    let target = self.asset_records.entry(asset).or_default();
    target.references.extend(record.references);
    target.replacements.extend(record.replacements);
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
      existing_record
        .replacements
        .append(&mut record.replacements);
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

    artifact.record_asset_hashes("main.aaaa.js", ["aaaa".to_string(), "bbbb".to_string()]);

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
  fn merge_asset_record_creates_asset_record_and_reverse_index() {
    let mut artifact = RealContentHashArtifact::default();
    let mut record = AssetHashRecord::default();
    record
      .own_hashes
      .extend(["aaaa".to_string(), "bbbb".to_string()]);
    record.references.push(ContentHashReference {
      referenced_hash: "cccc".to_string(),
      owner_hash: Some("aaaa".to_string()),
      referenced_chunk: None,
      referenced_source_type: None,
      kind: ContentHashReferenceKind::Source,
    });
    record.replacements.push(ContentHashReplacement {
      old_hash: "dddd".to_string(),
      range: Some(10..14),
      kind: ContentHashReplacementKind::Filename,
    });

    artifact.merge_asset_record("main.js", record);

    let record = artifact
      .asset_records
      .get("main.js")
      .expect("merged record should exist");
    assert_eq!(record.own_hashes.len(), 2);
    assert!(record.own_hashes.contains("aaaa"));
    assert!(record.own_hashes.contains("bbbb"));
    assert_eq!(record.references.len(), 1);
    assert_eq!(record.references[0].referenced_hash, "cccc");
    assert_eq!(record.references[0].owner_hash.as_deref(), Some("aaaa"));
    assert_eq!(record.replacements.len(), 1);
    assert_eq!(record.replacements[0].old_hash, "dddd");
    assert_eq!(record.replacements[0].range, Some(10..14));
    assert_eq!(
      record.replacements[0].kind,
      ContentHashReplacementKind::Filename
    );
    assert_eq!(
      artifact
        .hash_to_assets
        .get("aaaa")
        .expect("first hash owner"),
      &vec!["main.js".to_string()]
    );
    assert_eq!(
      artifact
        .hash_to_assets
        .get("bbbb")
        .expect("second hash owner"),
      &vec!["main.js".to_string()]
    );
  }

  #[test]
  fn manifest_content_hash_helpers_separate_owned_hashes_from_filename_replacements() {
    let mut record = AssetHashRecord::default();

    record_manifest_owned_content_hash(&mut record, Some("abcdef123456"));

    assert!(record.own_hashes.contains("abcdef123456"));
    assert!(record.replacements.is_empty());
  }

  #[test]
  fn manifest_filename_content_hashes_use_rendered_asset_info_values() {
    let mut record = AssetHashRecord::default();
    let content_hashes = ["abcdef12".to_string(), "YWJjZGVm".to_string()];

    record_manifest_filename_content_hashes(&mut record, content_hashes.iter());

    assert!(record.own_hashes.contains("abcdef12"));
    assert!(record.own_hashes.contains("YWJjZGVm"));
    assert_eq!(record.replacements.len(), 2);
    assert_eq!(record.replacements[0].old_hash, "abcdef12");
    assert_eq!(record.replacements[0].range, None);
    assert_eq!(
      record.replacements[0].kind,
      ContentHashReplacementKind::Filename
    );
    assert_eq!(record.replacements[1].old_hash, "YWJjZGVm");
    assert_eq!(record.replacements[1].range, None);
    assert_eq!(
      record.replacements[1].kind,
      ContentHashReplacementKind::Filename
    );
  }

  #[test]
  fn merge_asset_record_extends_existing_record_without_duplicate_reverse_index() {
    let mut artifact = RealContentHashArtifact::default();
    artifact.record_asset_hashes("main.js", ["aaaa".to_string(), "shared".to_string()]);
    artifact.record_reference(
      "main.js",
      "old-reference",
      Some("aaaa"),
      ContentHashReferenceMeta::default(),
    );
    artifact.record_replacement(
      "main.js",
      "old-replacement",
      None,
      ContentHashReplacementKind::Source,
    );
    let mut record = AssetHashRecord::default();
    record
      .own_hashes
      .extend(["bbbb".to_string(), "shared".to_string()]);
    record.references.push(ContentHashReference {
      referenced_hash: "new-reference".to_string(),
      owner_hash: Some("bbbb".to_string()),
      referenced_chunk: None,
      referenced_source_type: None,
      kind: ContentHashReferenceKind::Filename,
    });
    record.replacements.push(ContentHashReplacement {
      old_hash: "new-replacement".to_string(),
      range: Some(20..24),
      kind: ContentHashReplacementKind::Source,
    });

    artifact.merge_asset_record("main.js", record);

    let record = artifact
      .asset_records
      .get("main.js")
      .expect("merged record should exist");
    assert_eq!(record.own_hashes.len(), 3);
    assert!(record.own_hashes.contains("aaaa"));
    assert!(record.own_hashes.contains("bbbb"));
    assert!(record.own_hashes.contains("shared"));
    assert_eq!(record.references.len(), 2);
    assert_eq!(record.references[0].referenced_hash, "old-reference");
    assert_eq!(record.references[1].referenced_hash, "new-reference");
    assert_eq!(
      record.references[1].kind,
      ContentHashReferenceKind::Filename
    );
    assert_eq!(record.replacements.len(), 2);
    assert_eq!(record.replacements[0].old_hash, "old-replacement");
    assert_eq!(record.replacements[1].old_hash, "new-replacement");
    assert_eq!(record.replacements[1].range, Some(20..24));
    assert_eq!(
      artifact.hash_to_assets.get("aaaa").expect("old hash owner"),
      &vec!["main.js".to_string()]
    );
    assert_eq!(
      artifact.hash_to_assets.get("bbbb").expect("new hash owner"),
      &vec!["main.js".to_string()]
    );
    assert_eq!(
      artifact
        .hash_to_assets
        .get("shared")
        .expect("shared hash owner"),
      &vec!["main.js".to_string()]
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
      artifact
        .hash_to_assets
        .get("shared")
        .expect("shared hash owner"),
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
      artifact
        .hash_to_assets
        .get("shared")
        .expect("shared hash owner"),
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
