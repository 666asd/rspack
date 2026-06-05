use super::DenseDependencyIdMap;
use crate::{BoxDependency, DependencyId};

#[derive(Debug, Clone, Copy)]
enum DependencyStorageKind {
  Module,
  Presentational,
  Other,
}

#[derive(Debug, Clone, Default)]
pub struct DependencyStorage {
  kind_map: DenseDependencyIdMap<DependencyStorageKind>,
  module_dependencies: DenseDependencyIdMap<BoxDependency>,
  presentational_dependencies: DenseDependencyIdMap<BoxDependency>,
  other_dependencies: DenseDependencyIdMap<BoxDependency>,
}

impl DependencyStorage {
  #[inline]
  fn storage_kind(&self, dependency: &BoxDependency) -> DependencyStorageKind {
    if dependency.as_module_dependency().is_some() || dependency.as_context_dependency().is_some() {
      DependencyStorageKind::Module
    } else if dependency.as_dependency_code_generation().is_some() {
      DependencyStorageKind::Presentational
    } else {
      DependencyStorageKind::Other
    }
  }

  #[inline]
  pub fn insert(&mut self, key: DependencyId, value: BoxDependency) -> Option<BoxDependency> {
    self.remove(&key);
    let kind = self.storage_kind(&value);
    let inserted = match kind {
      DependencyStorageKind::Module => self.module_dependencies.insert(key, value),
      DependencyStorageKind::Presentational => self.presentational_dependencies.insert(key, value),
      DependencyStorageKind::Other => self.other_dependencies.insert(key, value),
    };
    self.kind_map.insert(key, kind);
    inserted
  }

  #[inline]
  fn get_storage(&self, kind: &DependencyStorageKind) -> &DenseDependencyIdMap<BoxDependency> {
    match kind {
      DependencyStorageKind::Module => &self.module_dependencies,
      DependencyStorageKind::Presentational => &self.presentational_dependencies,
      DependencyStorageKind::Other => &self.other_dependencies,
    }
  }

  #[inline]
  fn get_storage_mut(
    &mut self,
    kind: &DependencyStorageKind,
  ) -> &mut DenseDependencyIdMap<BoxDependency> {
    match kind {
      DependencyStorageKind::Module => &mut self.module_dependencies,
      DependencyStorageKind::Presentational => &mut self.presentational_dependencies,
      DependencyStorageKind::Other => &mut self.other_dependencies,
    }
  }

  #[inline]
  pub fn remove(&mut self, key: &DependencyId) -> Option<BoxDependency> {
    let kind = self.kind_map.remove(key)?;
    self.get_storage_mut(&kind).remove(key)
  }

  #[inline]
  pub fn get(&self, key: &DependencyId) -> Option<&BoxDependency> {
    let kind = self.kind_map.get(key)?;
    self.get_storage(kind).get(key)
  }

  #[inline]
  pub fn get_mut(&mut self, key: &DependencyId) -> Option<&mut BoxDependency> {
    let kind = self.kind_map.get(key).copied()?;
    self.get_storage_mut(&kind).get_mut(key)
  }

  #[inline]
  pub fn clear(&mut self) {
    self.kind_map.clear();
    self.module_dependencies.clear();
    self.presentational_dependencies.clear();
    self.other_dependencies.clear();
  }

  #[inline]
  pub fn iter(&self) -> impl Iterator<Item = (DependencyId, &BoxDependency)> {
    self
      .kind_map
      .iter()
      .filter_map(|(id, _kind)| self.get(&id).map(|dep| (id, dep)))
  }

  #[inline]
  pub fn module_dependencies(&self) -> impl Iterator<Item = (DependencyId, &BoxDependency)> {
    self
      .module_dependencies
      .iter()
      .filter_map(|(id, dep)| self.kind_map.get(&id).is_some().then_some((id, dep)))
  }

  #[inline]
  pub fn presentational_dependencies(
    &self,
  ) -> impl Iterator<Item = (DependencyId, &BoxDependency)> {
    self
      .presentational_dependencies
      .iter()
      .filter_map(|(id, dep)| self.kind_map.get(&id).is_some().then_some((id, dep)))
  }

  #[inline]
  pub fn is_module_dependency(&self, key: &DependencyId) -> bool {
    matches!(self.kind_map.get(key), Some(DependencyStorageKind::Module))
  }
}

#[cfg(test)]
mod tests {
  use rspack_cacheable::{cacheable, cacheable_dyn};

  use super::DependencyStorage;
  use crate::{
    AffectType, AsContextDependency, AsDependencyCodeGeneration, AsModuleDependency, Dependency,
    DependencyCodeGeneration, DependencyId, FactorizeInfo, ModuleDependency,
  };

  #[cacheable]
  #[derive(Debug, Clone)]
  struct TestModuleDependency {
    id: DependencyId,
    factorize_info: FactorizeInfo,
  }

  impl TestModuleDependency {
    fn new(id: DependencyId) -> Self {
      Self {
        id,
        factorize_info: FactorizeInfo::default(),
      }
    }
  }

  #[cacheable_dyn]
  impl Dependency for TestModuleDependency {
    fn id(&self) -> &DependencyId {
      &self.id
    }

    fn could_affect_referencing_module(&self) -> AffectType {
      AffectType::True
    }
  }

  #[cacheable_dyn]
  impl ModuleDependency for TestModuleDependency {
    fn request(&self) -> &str {
      "test"
    }

    fn factorize_info(&self) -> &crate::FactorizeInfo {
      &self.factorize_info
    }

    fn factorize_info_mut(&mut self) -> &mut crate::FactorizeInfo {
      &mut self.factorize_info
    }
  }

  impl AsContextDependency for TestModuleDependency {}

  impl AsDependencyCodeGeneration for TestModuleDependency {}

  #[cacheable]
  #[derive(Debug, Clone)]
  struct TestPresentationalDependency {
    id: DependencyId,
  }

  impl TestPresentationalDependency {
    fn new(id: DependencyId) -> Self {
      Self { id }
    }
  }

  #[cacheable_dyn]
  impl Dependency for TestPresentationalDependency {
    fn id(&self) -> &DependencyId {
      &self.id
    }

    fn could_affect_referencing_module(&self) -> AffectType {
      AffectType::True
    }
  }

  #[cacheable_dyn]
  impl DependencyCodeGeneration for TestPresentationalDependency {
    fn dependency_template(&self) -> Option<crate::DependencyTemplateType> {
      None
    }
  }

  impl AsContextDependency for TestPresentationalDependency {}
  impl AsModuleDependency for TestPresentationalDependency {}

  #[cacheable]
  #[derive(Debug, Clone)]
  struct TestOtherDependency {
    id: DependencyId,
  }

  impl TestOtherDependency {
    fn new(id: DependencyId) -> Self {
      Self { id }
    }
  }

  #[cacheable_dyn]
  impl Dependency for TestOtherDependency {
    fn id(&self) -> &DependencyId {
      &self.id
    }

    fn could_affect_referencing_module(&self) -> AffectType {
      AffectType::True
    }
  }

  impl AsContextDependency for TestOtherDependency {}
  impl AsModuleDependency for TestOtherDependency {}
  impl AsDependencyCodeGeneration for TestOtherDependency {}

  #[test]
  fn stores_module_and_presentational_dependencies_separately() {
    let mut storage = DependencyStorage::default();
    let module_id = DependencyId::from(1);
    let presentational_id = DependencyId::from(2);
    let other_id = DependencyId::from(3);

    storage.insert(module_id, Box::new(TestModuleDependency::new(module_id)));
    storage.insert(
      presentational_id,
      Box::new(TestPresentationalDependency::new(presentational_id)),
    );
    storage.insert(other_id, Box::new(TestOtherDependency::new(other_id)));

    assert!(storage.is_module_dependency(&module_id));
    assert!(!storage.is_module_dependency(&presentational_id));
    assert!(!storage.is_module_dependency(&other_id));
    assert!(storage.module_dependencies().any(|(id, _)| id == module_id));
    assert!(
      storage
        .presentational_dependencies()
        .any(|(id, _)| id == presentational_id)
    );
    assert_eq!(storage.iter().count(), 3);
    assert!(storage.get(&other_id).is_some());
  }
}
