use std::sync::Arc;

use rspack_cacheable::{
  cacheable,
  with::{AsCacheable, AsOption, AsVec},
};
use rspack_error::Diagnostic;
use rspack_paths::{ArcPath, ArcPathSet};

use super::{BoxDependency, DependencyId};

#[cacheable]
#[derive(Debug, Clone, Default)]
pub struct FactorizeInfo {
  /// Whether this value came from a factorization result.
  ///
  /// A default value is also stored on secondary dependencies after their
  /// factorization group's info has been moved to the first dependency; keep
  /// this flag so revocation can preserve the previous "no related ids"
  /// behavior for those defaults while still representing the common
  /// single-dependency result without storing its id.
  has_info: bool,
  /// Dependencies resolved by the same factorization task.
  ///
  /// Empty represents the overwhelmingly common "only this dependency" case.
  /// Callers that revoke a dependency already know the owner dependency id and
  /// can use that id without storing it in every hot dependency.
  #[cacheable(with=AsOption<AsVec<AsCacheable>>)]
  related_dep_ids: Option<Arc<[DependencyId]>>,
  #[cacheable(with=AsOption<AsVec<AsCacheable>>)]
  file_dependencies: Option<Arc<[ArcPath]>>,
  #[cacheable(with=AsOption<AsVec<AsCacheable>>)]
  context_dependencies: Option<Arc<[ArcPath]>>,
  #[cacheable(with=AsOption<AsVec<AsCacheable>>)]
  missing_dependencies: Option<Arc<[ArcPath]>>,
  diagnostics: Vec<Diagnostic>,
}

fn compact_path_set(paths: ArcPathSet) -> Option<Arc<[ArcPath]>> {
  if paths.is_empty() {
    None
  } else {
    Some(paths.into_iter().collect::<Vec<_>>().into())
  }
}

impl FactorizeInfo {
  pub fn new(
    diagnostics: Vec<Diagnostic>,
    related_dep_ids: Vec<DependencyId>,
    file_dependencies: ArcPathSet,
    context_dependencies: ArcPathSet,
    missing_dependencies: ArcPathSet,
  ) -> Self {
    Self {
      has_info: true,
      related_dep_ids: if related_dep_ids.len() > 1 {
        Some(related_dep_ids.into())
      } else {
        None
      },
      file_dependencies: compact_path_set(file_dependencies),
      context_dependencies: compact_path_set(context_dependencies),
      missing_dependencies: compact_path_set(missing_dependencies),
      diagnostics,
    }
  }

  pub fn get_from(dep: &BoxDependency) -> Option<&FactorizeInfo> {
    if let Some(d) = dep.as_context_dependency() {
      Some(d.factorize_info())
    } else if let Some(d) = dep.as_module_dependency() {
      Some(d.factorize_info())
    } else {
      None
    }
  }

  pub fn revoke(dep: &mut BoxDependency) -> Option<FactorizeInfo> {
    if let Some(d) = dep.as_context_dependency_mut() {
      Some(std::mem::take(d.factorize_info_mut()))
    } else if let Some(d) = dep.as_module_dependency_mut() {
      Some(std::mem::take(d.factorize_info_mut()))
    } else {
      None
    }
  }

  pub fn is_success(&self) -> bool {
    self.diagnostics.is_empty()
  }

  pub fn related_dep_ids(&self) -> &[DependencyId] {
    self.related_dep_ids.as_deref().unwrap_or_default()
  }

  pub fn related_dep_ids_for_revoke(&self, dep_id: DependencyId) -> Vec<DependencyId> {
    if let Some(related_dep_ids) = &self.related_dep_ids {
      related_dep_ids.to_vec()
    } else if self.has_info {
      vec![dep_id]
    } else {
      vec![]
    }
  }

  pub fn file_dependencies(&self) -> Option<&[ArcPath]> {
    self.file_dependencies.as_deref()
  }

  pub fn context_dependencies(&self) -> Option<&[ArcPath]> {
    self.context_dependencies.as_deref()
  }

  pub fn missing_dependencies(&self) -> Option<&[ArcPath]> {
    self.missing_dependencies.as_deref()
  }

  pub fn diagnostics(&self) -> &[Diagnostic] {
    &self.diagnostics
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn default_info_has_no_related_dependencies_for_revoke() {
    let dep_id = DependencyId::from(1);
    assert!(
      FactorizeInfo::default()
        .related_dep_ids_for_revoke(dep_id)
        .is_empty()
    );
  }

  #[test]
  fn single_dependency_info_uses_owner_for_revoke_without_storing_id() {
    let dep_id = DependencyId::from(1);
    let info = FactorizeInfo::new(
      vec![],
      vec![dep_id],
      Default::default(),
      Default::default(),
      Default::default(),
    );

    assert!(info.related_dep_ids().is_empty());
    assert_eq!(info.related_dep_ids_for_revoke(dep_id), vec![dep_id]);
  }

  #[test]
  fn multiple_dependency_info_keeps_related_ids_for_revoke() {
    let first = DependencyId::from(1);
    let second = DependencyId::from(2);
    let info = FactorizeInfo::new(
      vec![],
      vec![first, second],
      Default::default(),
      Default::default(),
      Default::default(),
    );

    assert_eq!(info.related_dep_ids(), &[first, second]);
    assert_eq!(info.related_dep_ids_for_revoke(first), vec![first, second]);
  }
}
