use std::{borrow::Cow, collections::VecDeque};

use concat_string::concat_string;
use rspack_core::{
  ChunkGraph, CssBuildInfo, CssExport, CssExportType, CssExports, CssModuleRenderCondition,
  DependencyId, DependencyType, GenerateContext, Module, ModuleArgument, ModuleInitFragments,
  RESERVED_IDENTIFIER, RuntimeGlobals, SourceType, TemplateContext, UsageState, UsedNameItem,
  css_module_render_conditions_identifier,
  rspack_sources::{
    BoxSource, ConcatSource, OriginalSource, RawStringSource, ReplaceSource, Source, SourceExt,
  },
  to_identifier,
};
use rspack_error::Result;
use rspack_util::{
  atom::Atom,
  fx_hash::{FxIndexMap, FxIndexSet},
  itoa, json_stringify, json_stringify_str,
};
use rustc_hash::FxHashSet as HashSet;

use crate::{
  dependency::CssImportDependency,
  parser_and_generator::{
    CssSourceBuilder, css_module_export_type, get_unused_local_ident, get_used_exports,
  },
  utils::{replace_css_module_id_placeholder, unescape},
};

pub fn update_css_exports(exports: &mut CssExports, name: &str, css_export: CssExport) -> bool {
  if let Some(existing) = exports.get_mut(name) {
    existing.insert(css_export)
  } else {
    exports
      .insert(name.into(), FxIndexSet::from_iter([css_export]))
      .is_none()
  }
}

pub(crate) struct CssModuleGenerator<'a, 'g> {
  source: &'a BoxSource,
  module: &'a dyn Module,
  css_build_info: &'a CssBuildInfo,
  generate_context: &'a mut GenerateContext<'g>,
  with_hmr: bool,
  export_type: Option<CssExportType>,
  exports_only: bool,
  es_module: bool,
  module_argument: Option<String>,
  concat_source: ConcatSource,
}

impl<'a, 'g> CssModuleGenerator<'a, 'g> {
  pub fn new(
    source: &'a BoxSource,
    module: &'a dyn Module,
    generate_context: &'a mut GenerateContext<'g>,
    with_hmr: bool,
    export_type: Option<CssExportType>,
    exports_only: bool,
    es_module: bool,
  ) -> Self {
    let css_build_info = module
      .build_info()
      .css
      .as_deref()
      .expect("CssParserAndGenerator should populate BuildInfo.css during parse");

    Self {
      source,
      module,
      css_build_info,
      generate_context,
      with_hmr,
      export_type,
      exports_only,
      es_module,
      module_argument: None,
      concat_source: Default::default(),
    }
  }

  fn module_argument(&mut self) -> &str {
    self.module_argument.get_or_insert_with(|| {
      self
        .generate_context
        .runtime_template
        .render_module_argument(ModuleArgument::Module)
    })
  }

  pub fn generate_javascript_source(mut self) -> Result<BoxSource> {
    match self.export_type {
      Some(CssExportType::Text) => {
        let css = self.css_text_expr_with_imports(true);
        let source = self.generate_css_text_exports(&css)?;
        self.concat_source.add(RawStringSource::from(source));
      }
      Some(CssExportType::CssStyleSheet) => {
        let css = self.css_text_expr_with_imports(true);
        let source = self.generate_css_style_sheet_exports(&css)?;
        self.concat_source.add(RawStringSource::from(source));
      }
      Some(CssExportType::Style) if !self.exports_only => {
        let imports = self.render_css_imports_for_style();
        let css_source = self.render_css_module_source();
        let css = self.css_text_expr(css_source, &[], false);
        self.concat_source.add(RawStringSource::from(imports));
        let inject_style = self.render_css_inject_style(&css);
        self.concat_source.add(RawStringSource::from(inject_style));
        self.generate_js_exports()?;
      }
      _ => {
        self.generate_js_exports()?;
      }
    }
    let generated_source = self.concat_source.source().into_string_lossy().into_owned();
    if self.module.get_source_map_kind().enabled() {
      Ok(OriginalSource::new(generated_source, self.module.identifier().as_str()).boxed())
    } else {
      Ok(RawStringSource::from(generated_source).boxed())
    }
  }

  fn child_generator<'b>(
    &'b mut self,
    source: &'b BoxSource,
    module: &'b dyn Module,
  ) -> CssModuleGenerator<'b, 'g> {
    CssModuleGenerator::new(
      source,
      module,
      self.generate_context,
      self.with_hmr,
      css_module_export_type(module),
      self.exports_only,
      self.es_module,
    )
  }

  pub(crate) fn render_css_module_source(&mut self) -> BoxSource {
    self.render_css_module_source_with_options(false)
  }

  fn render_css_module_source_for_text(&mut self) -> BoxSource {
    self.render_css_module_source_with_options(true)
  }

  fn render_css_module_source_with_options(&mut self, preserve_icss_symbols: bool) -> BoxSource {
    let module = self.module;
    let mut source = ReplaceSource::new(self.source.clone());
    let compilation = self.generate_context.compilation;
    let mut init_fragments = ModuleInitFragments::default();
    let mut context = TemplateContext {
      compilation,
      module,
      runtime: self.generate_context.runtime,
      init_fragments: &mut init_fragments,
      concatenation_scope: self.generate_context.concatenation_scope.take(),
      data: self.generate_context.data,
      runtime_template: self.generate_context.runtime_template,
    };

    let module_graph = compilation.get_module_graph();
    module.get_dependencies().iter().for_each(|id| {
      let dep = module_graph.dependency_by_id(id);
      if preserve_icss_symbols && matches!(dep.dependency_type(), DependencyType::CssIcssSymbol) {
        return;
      }

      if let Some(dependency) = dep.as_dependency_code_generation() {
        if let Some(template) = dependency
          .dependency_template()
          .and_then(|template_type| compilation.get_dependency_template(template_type))
        {
          template.render(dependency, &mut source, &mut context)
        } else {
          panic!(
            "Can not find dependency template of {:?}",
            dependency.dependency_template()
          );
        }
      }
    });

    if let Some(dependencies) = module.get_presentational_dependencies() {
      dependencies.iter().for_each(|dependency| {
        if let Some(template) = dependency
          .dependency_template()
          .and_then(|dependency_type| compilation.get_dependency_template(dependency_type))
        {
          template.render(dependency.as_ref(), &mut source, &mut context)
        } else {
          panic!(
            "Can not find dependency template of {:?}",
            dependency.dependency_template()
          );
        }
      });
    };

    self.generate_context.concatenation_scope = context.concatenation_scope.take();

    source.boxed()
  }

  fn css_text_expr_with_imports(&mut self, preserve_icss_symbols: bool) -> String {
    if !self.has_css_imports() {
      let css_source = if preserve_icss_symbols {
        self.render_css_module_source_for_text()
      } else {
        self.render_css_module_source()
      };
      return self.css_text_expr(css_source, &[], preserve_icss_symbols);
    }

    let mut seen = HashSet::default();
    let mut builder = CssSourceBuilder::new(false);
    let render_conditions = self
      .css_build_info
      .render_conditions()
      .cloned()
      .collect::<Vec<_>>();
    self.render_ordered_css_sources(
      &mut builder,
      &render_conditions,
      &mut seen,
      preserve_icss_symbols,
    );
    json_stringify_str(&builder.into_css_text())
  }

  fn has_css_imports(&self) -> bool {
    let module_graph = self.generate_context.compilation.get_module_graph();
    self.module.get_dependencies().iter().any(|dependency_id| {
      let dependency = module_graph.dependency_by_id(dependency_id);
      matches!(dependency.dependency_type(), DependencyType::CssImport)
    })
  }

  fn render_ordered_css_sources(
    &mut self,
    builder: &mut CssSourceBuilder,
    render_conditions: &[CssModuleRenderCondition],
    seen: &mut HashSet<rspack_collections::Identifier>,
    preserve_icss_symbols: bool,
  ) {
    let module = self.module;
    if !seen.insert(module.identifier()) {
      return;
    }

    self.render_css_import_sources(builder, seen, preserve_icss_symbols);
    let css_source = if preserve_icss_symbols {
      self.render_css_module_source_for_text()
    } else {
      self.render_css_module_source()
    };
    if !css_source.source().is_empty() {
      if self.css_build_info.has_charset {
        builder.set_has_charset();
      }
      builder.push_css_source(
        css_source,
        render_conditions,
        self.css_build_info.has_charset,
      );
    }
    seen.remove(&module.identifier());
  }

  fn render_css_import_sources(
    &mut self,
    builder: &mut CssSourceBuilder,
    seen: &mut HashSet<rspack_collections::Identifier>,
    preserve_icss_symbols: bool,
  ) {
    let compilation = self.generate_context.compilation;
    let module_graph = compilation.get_module_graph();
    let mut imported_modules = Vec::new();

    for dependency_id in self.module.get_dependencies() {
      let dependency = module_graph.dependency_by_id(dependency_id);
      if !matches!(dependency.dependency_type(), DependencyType::CssImport) {
        continue;
      }
      let Some(css_import_dep) = dependency.downcast_ref::<CssImportDependency>() else {
        panic!("dependency with type DependencyType::CssImport should only be CssImportDependency");
      };
      let Some(imported_module) = module_graph.module_graph_module_by_dependency_id(dependency_id)
      else {
        continue;
      };
      imported_modules.push((
        imported_module.module_identifier,
        css_import_dep
          .render_conditions()
          .cloned()
          .collect::<Vec<_>>(),
      ));
    }

    for (module_identifier, render_conditions) in imported_modules {
      let Some(imported_module) = module_graph.module_by_identifier(&module_identifier) else {
        continue;
      };
      let Some(imported_source) = imported_module.source() else {
        continue;
      };

      let mut child = self.child_generator(imported_source, imported_module.as_ref());
      child.render_ordered_css_sources(builder, &render_conditions, seen, preserve_icss_symbols);
    }
  }

  fn css_text_expr(
    &self,
    css_source: BoxSource,
    render_conditions: &[CssModuleRenderCondition],
    _preserve_icss_symbols: bool,
  ) -> String {
    let mut builder = CssSourceBuilder::new(self.css_build_info.has_charset);
    builder.push_css_source(
      css_source,
      render_conditions,
      self.css_build_info.has_charset,
    );
    json_stringify_str(&builder.into_css_text())
  }

  fn render_css_imports_for_style(&mut self) -> String {
    let mut visited_inlined_modules = HashSet::default();
    self.render_style_imports(&mut visited_inlined_modules)
  }

  fn render_style_imports(&mut self, visited_inlined_modules: &mut HashSet<String>) -> String {
    let compilation = self.generate_context.compilation;
    let module_graph = compilation.get_module_graph();
    let require = self
      .generate_context
      .runtime_template
      .render_runtime_globals(&RuntimeGlobals::REQUIRE);
    let mut code = String::new();

    for dependency_id in self.module.get_dependencies() {
      let dependency = module_graph.dependency_by_id(dependency_id);
      if !matches!(dependency.dependency_type(), DependencyType::CssImport) {
        continue;
      }
      let Some(css_import_dep) = dependency.downcast_ref::<CssImportDependency>() else {
        panic!("dependency with type DependencyType::CssImport should only be CssImportDependency");
      };

      let Some(imported_module) = module_graph.module_graph_module_by_dependency_id(dependency_id)
      else {
        continue;
      };

      let Some(module_id) = ChunkGraph::get_module_id(
        &compilation.module_ids_artifact,
        imported_module.module_identifier,
      ) else {
        continue;
      };

      let Some(imported_module) =
        module_graph.module_by_identifier(&imported_module.module_identifier)
      else {
        continue;
      };

      let render_conditions = css_import_dep
        .render_conditions()
        .cloned()
        .collect::<Vec<_>>();
      if is_style_export_css_module(imported_module.as_ref())
        && self.css_build_info.render_conditions().next().is_none()
        && render_conditions.is_empty()
      {
        code.push_str(&concat_string!(
          require,
          "(",
          json_stringify(module_id),
          ");\n"
        ));
        continue;
      }

      let Some(source) = imported_module.source() else {
        continue;
      };
      let render_conditions_key = css_render_conditions_key(&render_conditions);
      let inlined_module_key = concat_string!(
        imported_module.identifier().as_str(),
        "|",
        render_conditions_key
      );
      if !visited_inlined_modules.insert(inlined_module_key) {
        continue;
      }

      let mut child = self.child_generator(source, imported_module.as_ref());
      code.push_str(&child.render_style_imports(visited_inlined_modules));
      let css_source = child.render_css_module_source();
      let css = child.css_text_expr(css_source, &render_conditions, false);
      let style_module_id = if render_conditions_key.is_empty() {
        module_id.to_string()
      } else {
        concat_string!(module_id.to_string(), "|", render_conditions_key)
      };
      code.push_str(&self.render_inject_style_call(json_stringify_str(&style_module_id), &css));
    }

    code
  }

  fn render_css_inject_style(&mut self, css: &str) -> String {
    self
      .generate_context
      .runtime_template
      .runtime_requirements_mut()
      .insert(RuntimeGlobals::CSS_INJECT_STYLE);

    let module_id = ChunkGraph::get_module_id(
      &self.generate_context.compilation.module_ids_artifact,
      self.module.identifier(),
    )
    .map_or_else(
      || {
        self
          .module
          .readable_identifier(&self.generate_context.compilation.options.context)
          .into_owned()
      },
      |id| id.to_string(),
    );

    self.render_inject_style_call(json_stringify_str(&module_id), css)
  }

  fn render_inject_style_call(&mut self, module_id: String, css: &str) -> String {
    let css_inject_style = self
      .generate_context
      .runtime_template
      .render_runtime_globals(&RuntimeGlobals::CSS_INJECT_STYLE);
    concat_string!(css_inject_style, "(", module_id, ", ", css, ");\n")
  }

  fn generate_css_style_sheet_exports(&mut self, css: &str) -> Result<String> {
    let css_style_sheet_expr = self.render_css_style_sheet_expression(css);
    if self.generate_context.concatenation_scope.is_some() {
      self.concat_css_exports_with_default(Some(css_style_sheet_expr))?;
      return Ok(String::new());
    }

    let sheet_code = concat_string!("var __css_style_sheet = ", css_style_sheet_expr, ";\n");

    Ok(self.generate_css_default_exports(&sheet_code, "__css_style_sheet"))
  }

  fn generate_css_text_exports(&mut self, css: &str) -> Result<String> {
    if self.generate_context.concatenation_scope.is_some() {
      self.concat_css_exports_with_default(Some(css.to_string()))?;
      return Ok(String::new());
    }

    Ok(self.generate_css_default_exports("", css))
  }

  fn generate_css_default_exports(&mut self, prelude: &str, default_expr: &str) -> String {
    let module_argument = self.module_argument().to_string();
    let (ns_obj, left, right) = self.get_namespace_object_parts();

    if let Some((decl_name, exports_string)) = self.stringified_used_css_exports() {
      concat_string!(
        prelude,
        exports_string,
        "\n",
        ns_obj,
        left,
        module_argument,
        ".exports = Object.assign({}, ",
        decl_name,
        ")",
        right,
        ";\n",
        module_argument,
        ".exports.default = ",
        default_expr,
        ";\n"
      )
    } else if self.es_module {
      concat_string!(
        prelude,
        ns_obj,
        "(",
        module_argument,
        ".exports = {});\n",
        module_argument,
        ".exports.default = ",
        default_expr,
        ";\n"
      )
    } else {
      concat_string!(prelude, module_argument, ".exports = ", default_expr, ";\n")
    }
  }

  fn render_css_style_sheet_expression(&mut self, css: &str) -> String {
    self
      .generate_context
      .runtime_template
      .runtime_requirements_mut()
      .insert(RuntimeGlobals::CSS_STYLE_SHEET);
    let css_style_sheet = self
      .generate_context
      .runtime_template
      .render_runtime_globals(&RuntimeGlobals::CSS_STYLE_SHEET);
    concat_string!(css_style_sheet, "(", css, ")")
  }

  fn stringified_used_css_exports(&mut self) -> Option<(&'static str, String)> {
    if let Some(unused_exports) = get_unused_local_ident(
      self.css_build_info,
      self.module.identifier(),
      self.generate_context.runtime,
      &self.generate_context.compilation.exports_info_artifact,
    ) {
      self.generate_context.data.insert(unused_exports);
    }

    let exports = get_used_exports(
      self.css_build_info,
      self.module.identifier(),
      self.generate_context.runtime,
      &self.generate_context.compilation.exports_info_artifact,
    )?;

    Some(self.stringified_exports(exports))
  }

  fn get_namespace_object_parts(&mut self) -> (String, String, String) {
    if self.es_module {
      (
        self
          .generate_context
          .runtime_template
          .render_runtime_globals(&RuntimeGlobals::MAKE_NAMESPACE_OBJECT),
        "(".to_string(),
        ")".to_string(),
      )
    } else {
      (String::new(), String::new(), String::new())
    }
  }

  fn generate_js_exports(&mut self) -> Result<()> {
    let module = self.module;
    let build_info = module.build_info();
    let css_build_info = build_info
      .css
      .as_deref()
      .expect("CSS modules should have CssBuildInfo");
    let exports_info_artifact = &self.generate_context.compilation.exports_info_artifact;

    if self.generate_context.concatenation_scope.is_some() {
      if let Some(exports) = get_used_exports(
        css_build_info,
        module.identifier(),
        self.generate_context.runtime,
        exports_info_artifact,
      ) {
        if let Some(unused_exports) = get_unused_local_ident(
          css_build_info,
          module.identifier(),
          self.generate_context.runtime,
          exports_info_artifact,
        ) {
          self.generate_context.data.insert(unused_exports);
        }

        self.concat_css_exports(exports)?;
      }
      return Ok(());
    }

    let exports_info = exports_info_artifact.get_exports_info_data(&module.identifier());
    let (ns_obj, left, right): (Cow<'_, str>, &str, &str) = if self.es_module
      && exports_info
        .other_exports_info()
        .get_used(self.generate_context.runtime)
        != UsageState::Unused
    {
      (
        Cow::Owned(
          self
            .generate_context
            .runtime_template
            .render_runtime_globals(&RuntimeGlobals::MAKE_NAMESPACE_OBJECT),
        ),
        "(",
        ")",
      )
    } else {
      (Cow::Borrowed(""), "", "")
    };

    let exports_str = if let Some(exports) = get_used_exports(
      css_build_info,
      module.identifier(),
      self.generate_context.runtime,
      exports_info_artifact,
    ) {
      if let Some(unused_exports) = get_unused_local_ident(
        css_build_info,
        module.identifier(),
        self.generate_context.runtime,
        exports_info_artifact,
      ) {
        self.generate_context.data.insert(unused_exports);
      }

      self.css_modules_exports_to_string(exports, &ns_obj, left, right)
    } else {
      let hmr_code = self.render_accept_hmr();
      let module_argument = self.module_argument();
      concat_string!(
        ns_obj,
        left,
        module_argument,
        ".exports = {}",
        right,
        ";\n",
        hmr_code
      )
    };

    self.concat_source.add(RawStringSource::from(exports_str));
    Ok(())
  }

  fn css_modules_exports_to_string<'b>(
    &mut self,
    exports: rspack_util::fx_hash::FxIndexMap<&'b str, &'b FxIndexSet<CssExport>>,
    ns_obj: &str,
    left: &str,
    right: &str,
  ) -> String {
    let (decl_name, exports_string) = self.stringified_exports(exports);
    let hmr_code = self.render_exports_hmr(decl_name);
    let module_argument = self.module_argument();

    concat_string!(
      exports_string,
      "\n",
      hmr_code,
      "\n",
      ns_obj,
      left,
      module_argument,
      ".exports = ",
      decl_name,
      right,
      ";\n"
    )
  }

  fn concat_css_exports<'b>(
    &mut self,
    exports: FxIndexMap<&'b str, &'b FxIndexSet<CssExport>>,
  ) -> Result<()> {
    self.concat_css_exports_inner(None, Some(exports))
  }

  fn concat_css_exports_with_default(&mut self, default_expr: Option<String>) -> Result<()> {
    if let Some(unused_exports) = get_unused_local_ident(
      self.css_build_info,
      self.module.identifier(),
      self.generate_context.runtime,
      &self.generate_context.compilation.exports_info_artifact,
    ) {
      self.generate_context.data.insert(unused_exports);
    }

    let exports = get_used_exports(
      self.css_build_info,
      self.module.identifier(),
      self.generate_context.runtime,
      &self.generate_context.compilation.exports_info_artifact,
    );
    self.concat_css_exports_inner(default_expr, exports)
  }

  fn concat_css_exports_inner<'b>(
    &mut self,
    default_expr: Option<String>,
    exports: Option<FxIndexMap<&'b str, &'b FxIndexSet<CssExport>>>,
  ) -> Result<()> {
    if self.generate_context.concatenation_scope.is_none() {
      return Ok(());
    }

    let module = self.module;
    let compilation = self.generate_context.compilation;
    let runtime = self.generate_context.runtime;
    let exports_info = compilation
      .exports_info_artifact
      .get_exports_info_data(&module.identifier());
    let mut state = CssConcatenationState::default();

    if let Some(default_expr) = default_expr {
      self.register_concat_default_export(&default_expr, &mut state, exports_info, runtime);
    }

    let Some(exports) = exports else {
      return Ok(());
    };

    for (key, elements) in exports {
      let export_info = exports_info.get_read_only_export_info(&Atom::from(key));
      let used_name = export_info.get_used_name(None, runtime);
      let used_name: Cow<'_, str> = match used_name {
        Some(UsedNameItem::Str(name)) => Cow::Owned(name.to_string()),
        _ => Cow::Borrowed(key),
      };

      let content = self.render_concat_export_content(elements, &mut state);
      self.register_concat_export(key, &content, &used_name, &mut state);
    }

    Ok(())
  }

  fn register_concat_default_export(
    &mut self,
    content: &str,
    state: &mut CssConcatenationState<'_>,
    exports_info: &rspack_core::ExportsInfoData,
    runtime: Option<&rspack_core::RuntimeSpec>,
  ) {
    let export_info = exports_info.get_read_only_export_info(&Atom::from("default"));
    let Some(UsedNameItem::Str(used_name)) = export_info.get_used_name(None, runtime) else {
      return;
    };
    self.register_concat_export("default", content, &used_name, state);
  }

  fn register_concat_export(
    &mut self,
    key: &str,
    content: &str,
    used_name: &str,
    state: &mut CssConcatenationState<'_>,
  ) {
    let mut identifier = to_identifier(used_name).into_owned();
    if RESERVED_IDENTIFIER.contains(identifier.as_str()) {
      identifier = concat_string!("_", identifier);
    }
    let base_identifier = identifier.clone();
    let mut i = 0;
    while state.used_identifiers.contains(&identifier) {
      let mut i_buffer = itoa::Buffer::new();
      let i_str = i_buffer.format(i);
      identifier = concat_string!(base_identifier, i_str);
      i += 1;
    }

    let export_source = concat_string!("var ", identifier, " = ", content, ";\n");
    self.concat_source.add(RawStringSource::from(export_source));
    state.used_identifiers.insert(identifier.clone());
    let Some(ref mut scope) = self.generate_context.concatenation_scope else {
      unreachable!();
    };
    scope.register_export(key.into(), identifier);
  }

  fn render_css_export_content(&mut self, elements: &FxIndexSet<CssExport>) -> String {
    let mut content = String::new();
    for CssExport {
      ident,
      from,
      id: _,
      orig_name: _,
    } in elements
    {
      let part = self.render_css_export_part(ident, from.as_deref());
      push_joined(&mut content, &part, " + \" \" + ");
    }
    content
  }

  fn render_css_export_part(&mut self, ident: &str, from: Option<&str>) -> String {
    match from {
      None => self.render_local_css_export(ident),
      Some(from_name) => self.render_standard_css_reexport(ident, from_name),
    }
  }

  fn render_local_css_export(&self, ident: &str) -> String {
    let ident =
      replace_css_module_id_placeholder(ident, self.generate_context.compilation, self.module);
    json_stringify_str(&ident)
  }

  fn render_standard_css_reexport(&mut self, ident: &str, from_name: &str) -> String {
    let compilation = self.generate_context.compilation;
    let module_graph = compilation.get_module_graph();
    let from = self
      .module
      .get_dependencies()
      .iter()
      .find_map(|id| {
        let dependency = module_graph.dependency_by_id(id);
        let request = if let Some(d) = dependency.as_module_dependency() {
          Some(d.request())
        } else {
          dependency.as_context_dependency().map(|d| d.request())
        };
        if let Some(request) = request
          && request == from_name
        {
          return module_graph.module_graph_module_by_dependency_id(id);
        }
        None
      })
      .expect("should have css from module");

    let from_exports_info = compilation
      .exports_info_artifact
      .get_exports_info_data(&from.module_identifier);
    let from_used_name = match from_exports_info
      .get_read_only_export_info(&Atom::from(ident))
      .get_used_name(None, self.generate_context.runtime)
    {
      Some(UsedNameItem::Str(name)) => json_stringify_str(&unescape(name.as_str())),
      _ => json_stringify_str(&unescape(ident)),
    };

    let from = json_stringify(
      ChunkGraph::get_module_id(&compilation.module_ids_artifact, from.module_identifier)
        .expect("should have module"),
    );
    concat_string!(
      self
        .generate_context
        .runtime_template
        .render_runtime_globals(&RuntimeGlobals::REQUIRE),
      "(",
      from,
      ")[",
      from_used_name,
      "]"
    )
  }

  fn render_concat_export_content<'b>(
    &mut self,
    elements: &'b FxIndexSet<CssExport>,
    state: &mut CssConcatenationState<'b>,
  ) -> String
  where
    'g: 'b,
  {
    let mut content = String::new();
    for CssExport {
      ident,
      from,
      id,
      orig_name: _,
    } in elements
    {
      let part = self.render_concat_export_part(ident, from.as_deref(), id.as_ref(), state);
      push_joined(&mut content, &part, " + \" \" + ");
    }
    content
  }

  fn render_concat_export_part<'b>(
    &mut self,
    ident: &'b str,
    from: Option<&str>,
    id: Option<&'b DependencyId>,
    state: &mut CssConcatenationState<'b>,
  ) -> String
  where
    'g: 'b,
  {
    match from {
      None => self.render_concat_local_export(ident),
      Some(from_name) => self.render_concat_reexport(ident, from_name, id, state),
    }
  }

  fn render_concat_local_export(&self, ident: &str) -> String {
    let ident =
      replace_css_module_id_placeholder(ident, self.generate_context.compilation, self.module);
    json_stringify_str(&ident)
  }

  fn render_concat_reexport<'b>(
    &mut self,
    ident: &'b str,
    from_name: &str,
    id: Option<&'b DependencyId>,
    state: &mut CssConcatenationState<'b>,
  ) -> String
  where
    'g: 'b,
  {
    let compilation = self.generate_context.compilation;
    let module = self.module;
    let runtime = self.generate_context.runtime;
    let module_graph = compilation.get_module_graph();
    let current_module_identifier = module.identifier();
    let chunk_graph = &compilation.build_chunk_graph_artifact.chunk_graph;
    let current_module_chunks =
      if chunk_graph.get_number_of_module_chunks(current_module_identifier) > 0 {
        Some(chunk_graph.get_module_chunks(current_module_identifier))
      } else {
        None
      };
    let candidate_priority = |target: &dyn Module| {
      let target_identifier = target.identifier();
      let supports_javascript = target
        .source_types(module_graph)
        .contains(&SourceType::JavaScript);
      let shares_chunk = current_module_chunks.is_some_and(|current_chunks| {
        chunk_graph.get_number_of_module_chunks(target_identifier) > 0
          && chunk_graph
            .get_module_chunks(target_identifier)
            .iter()
            .any(|chunk| current_chunks.contains(chunk))
      });
      (
        supports_javascript,
        shares_chunk,
        ChunkGraph::get_module_id(&compilation.module_ids_artifact, target_identifier).is_some(),
      )
    };
    let find_target_module = |dep_id: &DependencyId| {
      module_graph
        .get_module_by_dependency_id(dep_id)
        .map(|target| {
          let priority = candidate_priority(target.as_ref());
          (target, priority)
        })
    };
    let from = id
      .and_then(find_target_module)
      .or_else(|| {
        module
          .get_dependencies()
          .iter()
          .filter(|dep_id| {
            let dependency = module_graph.dependency_by_id(dep_id);
            let request = if let Some(d) = dependency.as_module_dependency() {
              Some(d.request())
            } else {
              dependency.as_context_dependency().map(|d| d.request())
            };
            request == Some(from_name)
          })
          .filter_map(find_target_module)
          .max_by_key(|(_, priority)| *priority)
      })
      .map(|(target, _)| target)
      .and_then(|target| {
        if target
          .source_types(module_graph)
          .contains(&SourceType::JavaScript)
        {
          Some(target)
        } else {
          let target_name_for_condition = target.name_for_condition();
          module_graph
            .modules()
            .filter_map(|(_, candidate)| {
              (candidate.name_for_condition() == target_name_for_condition
                && candidate
                  .source_types(module_graph)
                  .contains(&SourceType::JavaScript))
              .then_some(candidate)
            })
            .max_by_key(|candidate| candidate_priority(candidate.as_ref()))
            .or(Some(target))
        }
      })
      .expect("should have css from module");

    let from_exports_info = compilation
      .exports_info_artifact
      .get_exports_info_data(&from.identifier());
    if !from
      .source_types(module_graph)
      .contains(&SourceType::JavaScript)
    {
      let from_css_build_info = from
        .build_info()
        .css
        .as_deref()
        .expect("CssParserAndGenerator should populate BuildInfo.css during parse");
      let resolved = state
        .resolve_static_export(compilation, from.as_ref(), from_css_build_info, ident)
        .expect("should resolve static css export");
      json_stringify_str(&resolved)
    } else {
      let from_used_name = match from_exports_info
        .get_read_only_export_info(&Atom::from(ident))
        .get_used_name(None, runtime)
      {
        Some(UsedNameItem::Str(name)) => json_stringify_str(name.as_str()),
        _ => json_stringify_str(ident),
      };

      let from = json_stringify(
        ChunkGraph::get_module_id(&compilation.module_ids_artifact, from.identifier())
          .expect("should have module"),
      );
      concat_string!(
        self
          .generate_context
          .runtime_template
          .render_runtime_globals(&RuntimeGlobals::REQUIRE),
        "(",
        from,
        ")[",
        from_used_name,
        "]"
      )
    }
  }

  fn stringified_exports<'b>(
    &mut self,
    exports: FxIndexMap<&'b str, &'b FxIndexSet<CssExport>>,
  ) -> (&'static str, String) {
    let module = self.module;
    let mut stringified_exports = String::new();

    for (key, elements) in exports {
      let used_name: Cow<'_, str> = {
        let exports_info = self
          .generate_context
          .compilation
          .exports_info_artifact
          .get_exports_info_data(&module.identifier());
        let export_info = exports_info.get_read_only_export_info(&Atom::from(key));
        match export_info.get_used_name(None, self.generate_context.runtime) {
          Some(UsedNameItem::Str(name)) => Cow::Owned(name.to_string()),
          _ => Cow::Borrowed(key),
        }
      };

      stringified_exports.push_str("  ");
      stringified_exports.push_str(&json_stringify_str(&used_name));
      stringified_exports.push_str(": ");
      stringified_exports.push_str(&self.render_css_export_content(elements));

      stringified_exports.push_str(",\n");
    }

    let decl_name = "exports";
    let exports_source = concat_string!("var ", decl_name, " = {\n", stringified_exports, "};");
    (decl_name, exports_source)
  }

  fn render_exports_hmr<'b>(&mut self, decl_name: &str) -> Cow<'b, str> {
    let with_hmr = self.with_hmr;
    let accept = self.render_accept_hmr();
    let module_argument = self.module_argument();

    if with_hmr {
      Cow::Owned(format!(
        "// only invalidate when locals change
var stringified_exports = JSON.stringify({decl_name});
if ({module_argument}.hot.data && {module_argument}.hot.data.exports && {module_argument}.hot.data.exports != stringified_exports) {{
  {module_argument}.hot.invalidate();
}} else {{
  {accept}}}
{module_argument}.hot.dispose(function(data) {{ data.exports = stringified_exports; }});"
      ))
    } else {
      Cow::Borrowed("")
    }
  }

  fn render_accept_hmr(&mut self) -> String {
    let with_hmr = self.with_hmr;
    let module_argument = self.module_argument();
    if with_hmr {
      concat_string!(module_argument, ".hot.accept();\n")
    } else {
      Default::default()
    }
  }
}

#[derive(Default)]
struct CssConcatenationState<'a> {
  used_identifiers: HashSet<String>,
  seen_static_exports: HashSet<(rspack_core::ModuleIdentifier, &'a str)>,
  static_export_queue: VecDeque<StaticCssExportFrame<'a>>,
}

struct StaticCssExportFrame<'a> {
  module: &'a dyn Module,
  css_build_info: &'a CssBuildInfo,
  export_name: &'a str,
  next_index: usize,
  resolved: String,
}

impl<'a> CssConcatenationState<'a> {
  fn resolve_static_export(
    &mut self,
    compilation: &'a rspack_core::Compilation,
    module: &'a dyn Module,
    css_build_info: &'a CssBuildInfo,
    export_name: &'a str,
  ) -> Option<String> {
    self.seen_static_exports.clear();
    self.static_export_queue.clear();

    self.push_static_export_frame(module, css_build_info, export_name)?;

    let module_graph = compilation.get_module_graph();
    while let Some(step) = self.next_static_export_step() {
      match step {
        StaticCssExportStep::Complete(resolved) => {
          if let Some(parent) = self.static_export_queue.back_mut() {
            if !resolved.is_empty() {
              push_joined(&mut parent.resolved, &resolved, " ");
            }
          } else {
            return (!resolved.is_empty()).then_some(resolved);
          }
        }
        StaticCssExportStep::Resolve { module, css_export } => match css_export.from.as_deref() {
          None => {
            let value = replace_css_module_id_placeholder(&css_export.ident, compilation, module);
            if let Some(frame) = self.static_export_queue.back_mut() {
              push_joined(&mut frame.resolved, value.as_ref(), " ");
            }
          }
          Some(from_request) => {
            let Some(target_identifier) =
              find_static_export_target(compilation, module, from_request, css_export.id.as_ref())
            else {
              continue;
            };
            let Some(target_module) = module_graph.module_by_identifier(&target_identifier) else {
              continue;
            };
            let target_css_build_info = target_module
              .build_info()
              .css
              .as_deref()
              .expect("CssParserAndGenerator should populate BuildInfo.css during parse");
            let _ = self.push_static_export_frame(
              target_module.as_ref(),
              target_css_build_info,
              &css_export.ident,
            );
          }
        },
      }
    }

    None
  }

  fn push_static_export_frame(
    &mut self,
    module: &'a dyn Module,
    css_build_info: &'a CssBuildInfo,
    export_name: &'a str,
  ) -> Option<()> {
    css_build_info.exports.get(export_name)?;
    let module_identifier = module.identifier();
    if !self
      .seen_static_exports
      .insert((module_identifier, export_name))
    {
      return None;
    }
    self.static_export_queue.push_back(StaticCssExportFrame {
      module,
      css_build_info,
      export_name,
      next_index: 0,
      resolved: String::new(),
    });
    Some(())
  }

  fn next_static_export_step(&mut self) -> Option<StaticCssExportStep<'a>> {
    let frame = self.static_export_queue.back_mut()?;
    if let Some(css_export) = frame
      .css_build_info
      .exports
      .get(frame.export_name)
      .and_then(|elements| elements.get_index(frame.next_index))
    {
      frame.next_index += 1;
      Some(StaticCssExportStep::Resolve {
        module: frame.module,
        css_export,
      })
    } else {
      Some(StaticCssExportStep::Complete(
        self
          .static_export_queue
          .pop_back()
          .expect("queue should have current frame")
          .resolved,
      ))
    }
  }
}

enum StaticCssExportStep<'a> {
  Resolve {
    module: &'a dyn Module,
    css_export: &'a CssExport,
  },
  Complete(String),
}

fn push_joined(target: &mut String, value: &str, separator: &str) {
  if !target.is_empty() {
    target.push_str(separator);
  }
  target.push_str(value);
}

fn css_render_conditions_key(conditions: &[CssModuleRenderCondition]) -> String {
  css_module_render_conditions_identifier(conditions).unwrap_or_default()
}

fn is_style_export_css_module(module: &dyn Module) -> bool {
  matches!(css_module_export_type(module), Some(CssExportType::Style))
}

fn find_static_export_target(
  compilation: &rspack_core::Compilation,
  module: &dyn Module,
  from_request: &str,
  id: Option<&DependencyId>,
) -> Option<rspack_core::ModuleIdentifier> {
  let module_graph = compilation.get_module_graph();
  id.and_then(|id| {
    module_graph
      .get_module_by_dependency_id(id)
      .map(|module| module.identifier())
  })
  .or_else(|| {
    module.get_dependencies().iter().find_map(|id| {
      let dependency = module_graph.dependency_by_id(id);
      let request = dependency
        .as_module_dependency()
        .map(|dep| dep.request())
        .or_else(|| dependency.as_context_dependency().map(|dep| dep.request()));
      (request == Some(from_request)).then(|| {
        module_graph
          .get_module_by_dependency_id(id)
          .map(|module| module.identifier())
      })?
    })
  })
}
