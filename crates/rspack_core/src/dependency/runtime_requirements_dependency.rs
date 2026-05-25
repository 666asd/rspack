use rspack_cacheable::{cacheable, cacheable_dyn};
use rspack_util::ext::DynHash;

use crate::{
  CodeGenerationRuntimeRequirementsWrite, Compilation, DependencyCodeGeneration, DependencyRange,
  DependencyTemplate, DependencyTemplateType, RuntimeGlobals, RuntimeSpec, TemplateContext,
  TemplateReplaceSource,
};

#[cacheable]
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum RuntimeRequirementsDependencyMode {
  #[default]
  Normal,
  Call,
  AddOnly,
  Write,
}

#[cacheable]
#[derive(Debug, Clone)]
pub struct RuntimeRequirementsDependency {
  pub range: DependencyRange,
  pub runtime_requirements: RuntimeGlobals,
  pub mode: RuntimeRequirementsDependencyMode,
}

#[cacheable_dyn]
impl DependencyCodeGeneration for RuntimeRequirementsDependency {
  fn dependency_template(&self) -> Option<DependencyTemplateType> {
    Some(RuntimeRequirementsDependencyTemplate::template_type())
  }

  fn update_hash(
    &self,
    hasher: &mut dyn std::hash::Hasher,
    _compilation: &Compilation,
    _runtime: Option<&RuntimeSpec>,
  ) {
    self.range.dyn_hash(hasher);
    self.runtime_requirements.dyn_hash(hasher);
    self.mode.dyn_hash(hasher);
  }
}

impl RuntimeRequirementsDependency {
  pub fn new(range: DependencyRange, runtime_requirements: RuntimeGlobals) -> Self {
    Self {
      range,
      runtime_requirements,
      mode: RuntimeRequirementsDependencyMode::Normal,
    }
  }
  pub fn call(range: DependencyRange, runtime_requirements: RuntimeGlobals) -> Self {
    Self {
      range,
      runtime_requirements,
      mode: RuntimeRequirementsDependencyMode::Call,
    }
  }
  pub fn add_only(runtime_requirements: RuntimeGlobals) -> Self {
    Self {
      range: DependencyRange::default(),
      runtime_requirements,
      mode: RuntimeRequirementsDependencyMode::AddOnly,
    }
  }
  pub fn write(runtime_requirements: RuntimeGlobals) -> Self {
    Self {
      range: DependencyRange::default(),
      runtime_requirements,
      mode: RuntimeRequirementsDependencyMode::Write,
    }
  }
}

#[cacheable]
#[derive(Debug, Clone, Default)]
pub struct RuntimeRequirementsDependencyTemplate;

impl RuntimeRequirementsDependencyTemplate {
  pub fn template_type() -> DependencyTemplateType {
    DependencyTemplateType::Custom("RuntimeRequirementsDependency")
  }
}

impl DependencyTemplate for RuntimeRequirementsDependencyTemplate {
  fn render(
    &self,
    dep: &dyn DependencyCodeGeneration,
    source: &mut TemplateReplaceSource,
    code_generatable_context: &mut TemplateContext,
  ) {
    let dep = dep
      .as_any()
      .downcast_ref::<RuntimeRequirementsDependency>()
      .expect(
        "RuntimeRequirementsDependencyTemplate should be used for RuntimeRequirementsDependency",
      );

    if matches!(dep.mode, RuntimeRequirementsDependencyMode::AddOnly) {
      code_generatable_context
        .runtime_template
        .runtime_requirements_mut()
        .insert(dep.runtime_requirements);
      return;
    }

    if matches!(dep.mode, RuntimeRequirementsDependencyMode::Write) {
      let mut runtime_requirements_write = code_generatable_context
        .data
        .get::<CodeGenerationRuntimeRequirementsWrite>()
        .cloned()
        .unwrap_or_default();
      runtime_requirements_write
        .runtime_requirements
        .insert(dep.runtime_requirements);
      code_generatable_context
        .data
        .insert(runtime_requirements_write);
      return;
    }

    let mut content = code_generatable_context
      .runtime_template
      .render_runtime_globals(&dep.runtime_requirements);

    if matches!(dep.mode, RuntimeRequirementsDependencyMode::Call) {
      content = format!("{content}()");
    }

    source.replace(dep.range.start, dep.range.end, content, None);
  }
}
