use std::sync::LazyLock;

use rspack_cacheable::cacheable;
use rspack_error::Diagnostic;
use rspack_paths::ArcPathSet;

use super::{BoxDependency, DependencyId};

static EMPTY_ARC_PATH_SET: LazyLock<ArcPathSet> = LazyLock::new(ArcPathSet::default);

#[cacheable]
#[derive(Debug, Clone, Default)]
pub struct FactorizeInfo {
  /// Whether this value came from a factorization result.
  ///
  /// A default value is also stored on secondary dependencies after their
  /// factorization group's info has been moved to the first dependency; keep
  /// this flag so revocation can preserve the previous "no related ids"
  /// behavior for those defaults.
  has_info: bool,
  /// Dependencies resolved by the same factorization task.
  ///
  /// `None` represents the overwhelmingly common "only this dependency" case.
  /// Callers that revoke a dependency already know the owner dependency id and
  /// can use that id when this field is absent.
  related_dep_ids: Option<Box<[DependencyId]>>,
  file_dependencies: Option<Box<ArcPathSet>>,
  context_dependencies: Option<Box<ArcPathSet>>,
  missing_dependencies: Option<Box<ArcPathSet>>,
  diagnostics: Option<Box<[Diagnostic]>>,
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
      related_dep_ids: (related_dep_ids.len() > 1).then_some(related_dep_ids.into_boxed_slice()),
      file_dependencies: (!file_dependencies.is_empty()).then_some(Box::new(file_dependencies)),
      context_dependencies: (!context_dependencies.is_empty())
        .then_some(Box::new(context_dependencies)),
      missing_dependencies: (!missing_dependencies.is_empty())
        .then_some(Box::new(missing_dependencies)),
      diagnostics: (!diagnostics.is_empty()).then_some(diagnostics.into_boxed_slice()),
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
    self.diagnostics.is_none()
  }

  pub fn related_dep_ids(&self) -> &[DependencyId] {
    self
      .related_dep_ids
      .as_ref()
      .map_or(&[], |related_dep_ids| related_dep_ids.as_ref())
  }

  pub fn related_dep_ids_for_revoke(&self, dep_id: DependencyId) -> Vec<DependencyId> {
    let related_dep_ids = self.related_dep_ids();
    if !related_dep_ids.is_empty() {
      related_dep_ids.to_vec()
    } else if self.has_info {
      vec![dep_id]
    } else {
      vec![]
    }
  }

  pub fn file_dependencies(&self) -> &ArcPathSet {
    self
      .file_dependencies
      .as_ref()
      .map_or(&EMPTY_ARC_PATH_SET, |file_dependencies| {
        file_dependencies.as_ref()
      })
  }

  pub fn context_dependencies(&self) -> &ArcPathSet {
    self
      .context_dependencies
      .as_ref()
      .map_or(&EMPTY_ARC_PATH_SET, |context_dependencies| {
        context_dependencies.as_ref()
      })
  }

  pub fn missing_dependencies(&self) -> &ArcPathSet {
    self
      .missing_dependencies
      .as_ref()
      .map_or(&EMPTY_ARC_PATH_SET, |missing_dependencies| {
        missing_dependencies.as_ref()
      })
  }

  pub fn diagnostics(&self) -> &[Diagnostic] {
    self
      .diagnostics
      .as_ref()
      .map_or(&[], |diagnostics| diagnostics.as_ref())
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
