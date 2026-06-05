use std::{fmt::Debug, sync::Arc};

use dyn_clone::{DynClone, clone_trait_object};
use rspack_cacheable::cacheable_dyn;
use rspack_sources::ReplaceSource;
use rspack_util::ext::AsAny;
use rustc_hash::FxHashMap as HashMap;

use crate::{
  ChunkInitFragments, CodeGenerationData, Compilation, ConcatenationScope, DependencyType, Module,
  ModuleCodeTemplate, ModuleInitFragments, RuntimeSpec,
};

pub struct TemplateContext<'a, 'b, 'c> {
  pub compilation: &'a Compilation,
  pub module: &'a dyn Module,
  pub init_fragments: &'a mut ModuleInitFragments<'b>,
  pub runtime: Option<&'a RuntimeSpec>,
  pub concatenation_scope: Option<&'c mut ConcatenationScope>,
  pub data: &'a mut CodeGenerationData,
  pub runtime_template: &'a mut ModuleCodeTemplate,
}

impl TemplateContext<'_, '_, '_> {
  pub fn chunk_init_fragments(&mut self) -> &mut ChunkInitFragments {
    let data_fragments = self.data.get::<ChunkInitFragments>();
    if data_fragments.is_some() {
      self
        .data
        .get_mut::<ChunkInitFragments>()
        .expect("should have chunk_init_fragments")
    } else {
      self.data.insert(ChunkInitFragments::default());
      self
        .data
        .get_mut::<ChunkInitFragments>()
        .expect("should have chunk_init_fragments")
    }
  }
}

pub type TemplateReplaceSource = ReplaceSource;

clone_trait_object!(DependencyCodeGeneration);

// Align with https://github.com/webpack/webpack/blob/671ac29d462e75a10c3fdfc785a4c153e41e749e/lib/DependencyCodeGeneration.js
#[cacheable_dyn]
pub trait DependencyCodeGeneration: Debug + DynClone + Sync + Send + AsAny {
  fn update_hash(
    &self,
    _hasher: &mut dyn std::hash::Hasher,
    _compilation: &Compilation,
    _runtime: Option<&RuntimeSpec>,
  ) {
  }

  fn dependency_template(&self) -> Option<DependencyTemplateType> {
    None
  }
}

pub type BoxDependencyTemplate = Box<dyn DependencyCodeGeneration>;

pub trait AsDependencyCodeGeneration {
  fn as_dependency_code_generation(&self) -> Option<&dyn DependencyCodeGeneration> {
    None
  }
}

impl<T: DependencyCodeGeneration> AsDependencyCodeGeneration for T {
  fn as_dependency_code_generation(&self) -> Option<&dyn DependencyCodeGeneration> {
    Some(self)
  }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum DependencyTemplateType {
  Dependency(DependencyType),
  Custom(&'static str),
}

#[derive(Debug, Default, Clone)]
pub struct DependencyTemplateStorage {
  dependency_templates: HashMap<DependencyType, Arc<dyn DependencyTemplate>>,
  custom_templates: HashMap<&'static str, Arc<dyn DependencyTemplate>>,
}

impl DependencyTemplateStorage {
  #[inline]
  pub fn insert(
    &mut self,
    template_type: DependencyTemplateType,
    template: Arc<dyn DependencyTemplate>,
  ) -> Option<Arc<dyn DependencyTemplate>> {
    match template_type {
      DependencyTemplateType::Dependency(dependency_type) => {
        self.dependency_templates.insert(dependency_type, template)
      }
      DependencyTemplateType::Custom(name) => self.custom_templates.insert(name, template),
    }
  }

  #[inline]
  pub fn get(
    &self,
    template_type: &DependencyTemplateType,
  ) -> Option<&Arc<dyn DependencyTemplate>> {
    match template_type {
      DependencyTemplateType::Dependency(dependency_type) => {
        self.dependency_templates.get(dependency_type)
      }
      DependencyTemplateType::Custom(name) => self.custom_templates.get(name),
    }
  }
}

pub trait DependencyTemplate: Debug + Sync + Send {
  fn render(
    &self,
    dep: &dyn DependencyCodeGeneration,
    source: &mut ReplaceSource,
    code_generatable_context: &mut TemplateContext,
  );
}
