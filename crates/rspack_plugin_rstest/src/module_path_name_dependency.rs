use rspack_cacheable::{cacheable, cacheable_dyn};
use rspack_core::{
  AsContextDependency, AsModuleDependency, DependencyCodeGeneration, DependencyRange,
  DependencyTemplate, DependencyTemplateType, DependencyType, InitFragmentExt, InitFragmentKey,
  InitFragmentStage, NormalInitFragment, TemplateContext, TemplateReplaceSource,
};
use rspack_util::json_stringify;
use swc_core::atoms::Atom;

#[cacheable]
#[derive(Debug, Clone, PartialEq)]
pub enum NameType {
  DirName,
  FileName,
}

#[cacheable]
#[derive(Debug, Clone)]
pub struct ModulePathNameDependency {
  r#type: NameType,
  range: DependencyRange,
}

impl ModulePathNameDependency {
  pub fn new(r#type: NameType, range: DependencyRange) -> Self {
    Self { r#type, range }
  }
}

#[cacheable_dyn]
impl DependencyCodeGeneration for ModulePathNameDependency {
  fn dependency_template(&self) -> Option<DependencyTemplateType> {
    Some(ModulePathNameDependencyTemplate::template_type())
  }
}

impl AsModuleDependency for ModulePathNameDependency {}
impl AsContextDependency for ModulePathNameDependency {}

#[cacheable]
#[derive(Debug, Clone, Default)]
pub struct ModulePathNameDependencyTemplate;

impl ModulePathNameDependencyTemplate {
  pub fn template_type() -> DependencyTemplateType {
    DependencyTemplateType::Dependency(DependencyType::RstestModulePath)
  }
}

impl DependencyTemplate for ModulePathNameDependencyTemplate {
  fn render(
    &self,
    dep: &dyn DependencyCodeGeneration,
    _source: &mut TemplateReplaceSource,
    code_generatable_context: &mut TemplateContext,
  ) {
    let TemplateContext {
      module,
      init_fragments,
      concatenation_scope,
      ..
    } = code_generatable_context;

    let m = module.as_normal_module();
    if let Some(m) = m {
      let resource_path = m.resource_resolved_data().path();

      let dep = dep
        .as_any()
        .downcast_ref::<ModulePathNameDependency>()
        .expect("ModulePathNameDependencyTemplate can only be applied to ModulePathNameDependency");

      if dep.r#type == NameType::FileName {
        if let Some(resource_path) = resource_path {
          let name = concatenation_scope
            .as_ref()
            .and_then(|scope| {
              scope
                .current_module
                .get_internal_name(&Atom::from("__filename"))
            })
            .cloned()
            .unwrap_or_else(|| Atom::from("__filename"));
          _source.replace(dep.range.start, dep.range.end, name.to_string(), None);
          let init = NormalInitFragment::new(
            format!(
              "const {name} = {};\n",
              json_stringify(&resource_path.as_std_path())
            ),
            InitFragmentStage::StageConstants,
            0,
            InitFragmentKey::Const(format!("rstest __filename {}", m.id())),
            None,
          );

          init_fragments.push(init.boxed());
        }
      } else if dep.r#type == NameType::DirName
        && let Some(resource_path) = resource_path
        && let Some(parent_path) = resource_path.parent()
      {
        let name = concatenation_scope
          .as_ref()
          .and_then(|scope| {
            scope
              .current_module
              .get_internal_name(&Atom::from("__dirname"))
          })
          .cloned()
          .unwrap_or_else(|| Atom::from("__dirname"));
        _source.replace(dep.range.start, dep.range.end, name.to_string(), None);
        // If the parent path is None, we use an empty string
        // to avoid issues with the path being undefined.
        let init = NormalInitFragment::new(
          format!(
            "const {name} = {};\n",
            json_stringify(parent_path.as_std_path())
          ),
          InitFragmentStage::StageConstants,
          0,
          InitFragmentKey::Const(format!("rstest __dirname {}", m.id())),
          None,
        );

        init_fragments.push(init.boxed());
      }
    }
  }
}
