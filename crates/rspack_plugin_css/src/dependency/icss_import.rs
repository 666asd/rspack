use std::collections::HashSet;

use rspack_cacheable::{cacheable, cacheable_dyn};
use rspack_core::{
  AsContextDependency, Compilation, Dependency, DependencyCategory, DependencyCodeGeneration,
  DependencyId, DependencyRange, DependencyTemplate, DependencyTemplateType, DependencyType,
  ExportsInfoArtifact, ExtendedReferencedExport, FactorizeInfo, Module, ModuleDependency,
  RuntimeSpec, TemplateContext, TemplateReplaceSource,
};
use rspack_util::atom::Atom;

use super::import::CssImportMode;

#[cacheable]
#[derive(Debug, Clone)]
pub struct CssIcssImportDependency {
  id: DependencyId,
  request: String,
  import_name: String,
  local_name: String,
  replace_range: Option<DependencyRange>,
  mode: Option<CssImportMode>,
  factorize_info: FactorizeInfo,
}

impl CssIcssImportDependency {
  pub fn new(
    request: String,
    import_name: String,
    local_name: String,
    replace_range: Option<DependencyRange>,
    mode: Option<CssImportMode>,
  ) -> Self {
    Self {
      id: DependencyId::new(),
      request,
      import_name,
      local_name,
      replace_range,
      mode,
      factorize_info: Default::default(),
    }
  }

  fn resolve_content(&self, compilation: &Compilation, module: &dyn Module) -> Option<String> {
    let module_graph = compilation.get_module_graph();
    let mut seen = HashSet::default();
    resolve_icss_export(
      module_graph,
      module,
      &self.request,
      &self.import_name,
      &mut seen,
    )
  }
}

fn resolve_icss_export(
  module_graph: &rspack_core::ModuleGraph,
  module: &dyn Module,
  request: &str,
  export_name: &str,
  seen: &mut HashSet<(rspack_core::ModuleIdentifier, String)>,
) -> Option<String> {
  let dep_id = module.get_dependencies().iter().find(|id| {
    module_graph
      .dependency_by_id(id)
      .as_module_dependency()
      .map(|dep| dep.request() == request)
      .unwrap_or(false)
  })?;

  let target = module_graph.get_module_by_dependency_id(dep_id)?;
  let target_identifier = target.identifier();
  if !seen.insert((target_identifier, export_name.to_string())) {
    return None;
  }

  let target_module = module_graph.module_by_identifier(&target_identifier)?;
  let exports = target_module.build_info().css_exports.as_ref()?;
  let elements = exports.get(export_name)?;

  let values = elements
    .iter()
    .filter_map(|css_export| match css_export.from.as_deref() {
      None => Some(css_export.ident.clone()),
      Some(from_request) => resolve_icss_export(
        module_graph,
        target_module.as_ref(),
        from_request,
        &css_export.ident,
        seen,
      ),
    })
    .collect::<Vec<_>>();

  if values.is_empty() {
    None
  } else {
    Some(values.join(" "))
  }
}

#[cacheable_dyn]
impl Dependency for CssIcssImportDependency {
  fn id(&self) -> &DependencyId {
    &self.id
  }

  fn category(&self) -> &DependencyCategory {
    match self.mode {
      Some(CssImportMode::Local) => &DependencyCategory::CssImportLocalModule,
      Some(CssImportMode::Global) => &DependencyCategory::CssImportGlobalModule,
      None => &DependencyCategory::CssImport,
    }
  }

  fn dependency_type(&self) -> &DependencyType {
    &DependencyType::CssIcssImport
  }

  fn range(&self) -> Option<DependencyRange> {
    self.replace_range
  }

  fn could_affect_referencing_module(&self) -> rspack_core::AffectType {
    rspack_core::AffectType::True
  }

  fn get_referenced_exports(
    &self,
    _module_graph: &rspack_core::ModuleGraph,
    _module_graph_cache: &rspack_core::ModuleGraphCacheArtifact,
    _exports_info_artifact: &ExportsInfoArtifact,
    _runtime: Option<&RuntimeSpec>,
  ) -> Vec<ExtendedReferencedExport> {
    vec![ExtendedReferencedExport::Array(vec![Atom::from(
      self.import_name.as_str(),
    )])]
  }
}

#[cacheable_dyn]
impl ModuleDependency for CssIcssImportDependency {
  fn request(&self) -> &str {
    &self.request
  }

  fn user_request(&self) -> &str {
    &self.request
  }

  fn factorize_info(&self) -> &FactorizeInfo {
    &self.factorize_info
  }

  fn factorize_info_mut(&mut self) -> &mut FactorizeInfo {
    &mut self.factorize_info
  }
}

#[cacheable_dyn]
impl DependencyCodeGeneration for CssIcssImportDependency {
  fn dependency_template(&self) -> Option<DependencyTemplateType> {
    Some(CssIcssImportDependencyTemplate::template_type())
  }
}

impl AsContextDependency for CssIcssImportDependency {}

#[cacheable]
#[derive(Debug, Clone, Default)]
pub struct CssIcssImportDependencyTemplate;

impl CssIcssImportDependencyTemplate {
  pub fn template_type() -> DependencyTemplateType {
    DependencyTemplateType::Dependency(DependencyType::CssIcssImport)
  }
}

impl DependencyTemplate for CssIcssImportDependencyTemplate {
  fn render(
    &self,
    dep: &dyn DependencyCodeGeneration,
    source: &mut TemplateReplaceSource,
    code_generatable_context: &mut TemplateContext,
  ) {
    let dep = dep
      .as_any()
      .downcast_ref::<CssIcssImportDependency>()
      .expect("CssIcssImportDependencyTemplate should be used for CssIcssImportDependency");

    let Some(range) = dep.replace_range else {
      return;
    };

    if let Some(content) = dep.resolve_content(
      code_generatable_context.compilation,
      code_generatable_context.module,
    ) {
      source.replace(range.start, range.end, content, None);
    }
  }
}
