use std::{borrow::Cow, collections::HashSet};

use rspack_core::{
  ChunkGraph, CssExport, CssExportType, CssExports, DependencyType, GenerateContext,
  InitFragmentRenderContext, Module, ModuleArgument, ModuleInitFragments, RESERVED_IDENTIFIER,
  RuntimeGlobals, TemplateContext, UsageState, UsedNameItem,
  rspack_sources::{BoxSource, ConcatSource, RawStringSource, ReplaceSource, SourceExt},
  to_identifier,
};
use rspack_util::{atom::Atom, fx_hash::FxIndexSet, itoa, json_stringify, json_stringify_str};

use crate::{
  dependency::{CssImportDependency, CssMedia, CssSupports},
  parser_and_generator::{
    CssExportsRef, CssParserAndGenerator, get_unused_local_ident, get_used_exports,
  },
  utils::unescape,
};

pub fn update_css_exports(exports: &mut CssExports, name: String, css_export: CssExport) -> bool {
  if let Some(existing) = exports.get_mut(&name) {
    existing.insert(css_export)
  } else {
    exports
      .insert(name, FxIndexSet::from_iter([css_export]))
      .is_none()
  }
}

pub(crate) struct CssGenerator<'a, 'g> {
  source: &'a BoxSource,
  module: &'a dyn Module,
  generate_context: &'a mut GenerateContext<'g>,
  with_hmr: bool,
  export_type: Option<CssExportType>,
  es_module: bool,
  exports_only: bool,
  module_argument: String,
  concat_source: ConcatSource,
}

impl<'a, 'g> CssGenerator<'a, 'g> {
  pub fn new(
    source: &'a BoxSource,
    module: &'a dyn Module,
    generate_context: &'a mut GenerateContext<'g>,
    with_hmr: bool,
    export_type: Option<CssExportType>,
    es_module: bool,
    exports_only: bool,
  ) -> Self {
    let runtime_template = generate_context.runtime_template();
    let module_argument = runtime_template.render_module_argument(ModuleArgument::Module);

    Self {
      source,
      module,
      generate_context,
      with_hmr,
      export_type,
      es_module,
      exports_only,
      module_argument,
      concat_source: Default::default(),
    }
  }

  fn es_module(&self) -> bool {
    self.es_module
  }

  fn module_argument(&self) -> &str {
    &self.module_argument
  }

  pub fn generate_javascript_source(mut self) -> BoxSource {
    let export_type = self.export_type.clone();
    let exports_only = self.exports_only;

    match export_type {
      Some(CssExportType::Style) if exports_only => {
        self.generate_js_exports();
      }
      Some(CssExportType::Style) => {
        let css_imports = self.render_css_imports_for_style();
        let css_js_string = self.stringify_css_source_for_javascript();

        self.concat_source.add(RawStringSource::from(css_imports));
        let inject_style = self.render_css_inject_style(&css_js_string);
        self.concat_source.add(RawStringSource::from(inject_style));
        self.generate_js_exports();
      }
      Some(CssExportType::CssStyleSheet) => {
        let css_js_string = self.stringify_css_source_for_javascript();
        let exports = self.generate_css_style_sheet_exports(&css_js_string);
        self.concat_source.add(RawStringSource::from(exports));
      }
      Some(CssExportType::Text) => {
        let css_js_string = self.stringify_css_source_for_javascript();
        let exports = self.generate_css_text_exports(&css_js_string);
        self.concat_source.add(RawStringSource::from(exports));
      }
      _ => {
        self.generate_js_exports();
      }
    };

    self.concat_source.boxed()
  }

  fn generate_js_exports(&mut self) {
    let module = self.module;
    let with_hmr = self.with_hmr;

    let build_info = module.build_info();
    if self.generate_context.concatenation_scope.is_some() {
      if let Some(ref exports) = build_info.css_exports {
        let exports_info_artifact = &self.generate_context.compilation.exports_info_artifact;
        if let Some(local_names) = &build_info.css_local_names {
          let unused_exports = get_unused_local_ident(
            exports,
            local_names,
            module.identifier(),
            self.generate_context.runtime,
            exports_info_artifact,
          );
          self.generate_context.data.insert(unused_exports);
        }
        let exports = get_used_exports(
          exports,
          module.identifier(),
          self.generate_context.runtime,
          exports_info_artifact,
        );

        self.css_modules_exports_to_concatenate_module_string(exports);
      }
    } else {
      let exports_info = self
        .generate_context
        .compilation
        .exports_info_artifact
        .get_exports_info_data(&module.identifier());
      let (ns_obj, left, right) = if self.es_module()
        && exports_info
          .other_exports_info()
          .get_used(self.generate_context.runtime)
          != UsageState::Unused
      {
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
      };
      let exports_str = if let Some(exports) = &build_info.css_exports {
        if let Some(local_names) = &build_info.css_local_names {
          let unused_exports = get_unused_local_ident(
            exports,
            local_names,
            module.identifier(),
            self.generate_context.runtime,
            &self.generate_context.compilation.exports_info_artifact,
          );
          self.generate_context.data.insert(unused_exports);
        }

        let exports = get_used_exports(
          exports,
          module.identifier(),
          self.generate_context.runtime,
          &self.generate_context.compilation.exports_info_artifact,
        );

        self.css_modules_exports_to_string(exports, &ns_obj, &left, &right)
      } else {
        let module_argument = self
          .generate_context
          .runtime_template
          .render_module_argument(ModuleArgument::Module);
        format!(
          "{}{}{module_argument}.exports = {{}}{};\n{}",
          &ns_obj,
          &left,
          &right,
          if with_hmr {
            format!("{module_argument}.hot.accept();\n")
          } else {
            Default::default()
          }
        )
      };

      self.concat_source.add(RawStringSource::from(exports_str))
    }
  }

  fn stringify_css_source_for_javascript(&mut self) -> String {
    self.stringify_css_source_for_module(self.source, self.module)
  }

  fn stringify_css_source_for_module(&mut self, source: &BoxSource, module: &dyn Module) -> String {
    let generate_context = &mut *self.generate_context;

    let mut source = ReplaceSource::new(source.clone());
    let compilation = generate_context.compilation;
    let mut init_fragments = ModuleInitFragments::default();
    let mut context = TemplateContext {
      compilation,
      module,
      runtime: generate_context.runtime,
      init_fragments: &mut init_fragments,
      concatenation_scope: generate_context.concatenation_scope.take(),
      data: generate_context.data,
      runtime_template: generate_context.runtime_template,
    };

    let module_graph = compilation.get_module_graph();
    module.get_dependencies().iter().for_each(|id| {
      let dep = module_graph.dependency_by_id(id);

      if let Some(dependency) = dep.as_dependency_code_generation() {
        if let Some(template) = dependency
          .dependency_template()
          .and_then(|template_type| compilation.get_dependency_template(template_type))
        {
          template.render(dependency, &mut source, &mut context)
        }
      }
    });

    for conn in module_graph.get_incoming_connections(&module.identifier()) {
      let dep = module_graph.dependency_by_id(&conn.dependency_id);

      if matches!(dep.dependency_type(), DependencyType::CssImport) {
        let Some(css_import_dep) = dep.downcast_ref::<CssImportDependency>() else {
          panic!(
            "dependency with type DependencyType::CssImport should only be CssImportDependency"
          );
        };

        if let Some(media) = css_import_dep.media() {
          context.data.insert(CssMedia(media.to_string()));
        }

        if let Some(supports) = css_import_dep.supports() {
          context.data.insert(CssSupports(supports.to_string()));
        }

        if let Some(layer) = css_import_dep.layer() {
          context.data.insert(layer.clone());
        }
      }
    }

    if let Some(dependencies) = module.get_presentational_dependencies() {
      dependencies.iter().for_each(|dependency| {
        if let Some(template) = dependency
          .dependency_template()
          .and_then(|dependency_type| compilation.get_dependency_template(dependency_type))
        {
          template.render(dependency.as_ref(), &mut source, &mut context)
        }
      });
    };

    generate_context.concatenation_scope = context.concatenation_scope.take();

    let css_source = source.boxed();
    let css_text = css_source
      .source()
      .into_string_lossy()
      .replace(crate::utils::AUTO_PUBLIC_PATH_PLACEHOLDER, "");

    json_stringify_str(&css_text)
  }

  fn render_css_imports_for_style(&mut self) -> String {
    let mut visited_non_style_modules = HashSet::default();
    self.render_css_imports_for_style_module(self.module, &mut visited_non_style_modules)
  }

  fn render_css_imports_for_style_module(
    &mut self,
    module: &dyn Module,
    visited_non_style_modules: &mut HashSet<rspack_collections::Identifier>,
  ) -> String {
    let compilation = self.generate_context.compilation;
    let module_graph = compilation.get_module_graph();
    let require = self
      .generate_context
      .runtime_template
      .render_runtime_globals(&RuntimeGlobals::REQUIRE);
    let mut code = String::new();

    for dependency_id in module.get_dependencies() {
      let dependency = module_graph.dependency_by_id(dependency_id);
      if !matches!(dependency.dependency_type(), DependencyType::CssImport) {
        continue;
      }

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

      if Self::is_style_export_css_module(imported_module.as_ref()) {
        code.push_str(&format!("{require}({});\n", json_stringify(module_id)));
        continue;
      }

      if !visited_non_style_modules.insert(imported_module.identifier()) {
        continue;
      }

      code.push_str(
        &self
          .render_css_imports_for_style_module(imported_module.as_ref(), visited_non_style_modules),
      );

      let Some(source) = imported_module.source() else {
        continue;
      };
      let css_js_string = self.stringify_css_source_for_module(source, imported_module.as_ref());
      code.push_str(
        &self.render_css_inject_style_by_module_id(json_stringify(module_id), &css_js_string),
      );
    }

    code
  }

  fn render_css_inject_style(&mut self, css_js_string: &str) -> String {
    let generate_context = &mut *self.generate_context;
    let module = self.module;

    generate_context
      .runtime_template
      .runtime_requirements_mut()
      .insert(RuntimeGlobals::CSS_INJECT_STYLE);

    let module_id = ChunkGraph::get_module_id(
      &generate_context.compilation.module_ids_artifact,
      module.identifier(),
    )
    .map(|id| id.to_string())
    .unwrap_or_default();

    self.render_css_inject_style_by_module_id(json_stringify_str(&module_id), css_js_string)
  }

  fn render_css_inject_style_by_module_id(
    &mut self,
    module_id: String,
    css_js_string: &str,
  ) -> String {
    format!(
      "{}({}, {});\n",
      self
        .generate_context
        .runtime_template
        .render_runtime_globals(&RuntimeGlobals::CSS_INJECT_STYLE),
      module_id,
      css_js_string
    )
  }

  fn is_style_export_css_module(module: &dyn Module) -> bool {
    module
      .as_normal_module()
      .and_then(|module| {
        module
          .parser_and_generator()
          .downcast_ref::<CssParserAndGenerator>()
      })
      .is_some_and(|parser_and_generator| {
        matches!(
          parser_and_generator.export_type(),
          Some(CssExportType::Style)
        )
      })
  }

  fn generate_css_style_sheet_exports(&mut self, css_js_string: &str) -> String {
    self
      .generate_context
      .runtime_template
      .runtime_requirements_mut()
      .insert(RuntimeGlobals::CSS_STYLE_SHEET);

    let module_argument = self
      .generate_context
      .runtime_template
      .render_module_argument(ModuleArgument::Module);
    let (ns_obj, left, right) = self.get_namespace_object_parts();
    let css_style_sheet_code = format!(
      "var __css_style_sheet = {}({});\n",
      self
        .generate_context
        .runtime_template
        .render_runtime_globals(&RuntimeGlobals::CSS_STYLE_SHEET),
      css_js_string
    );

    let source = if let Some((decl_name, exports_string)) = self.stringified_used_css_exports() {
      let hmr_code = self.render_exports_hmr(&decl_name);
      let mut code = format!(
        "{css_style_sheet_code}{exports_string}\n{hmr_code}\n{ns_obj}{left}{module_argument}.exports = Object.assign({{}}, {decl_name})",
      );
      code.push_str(&right);
      code.push_str(";\n");
      code.push_str(&module_argument);
      code.push_str(".exports.default = __css_style_sheet;\n");
      code
    } else {
      let mut code = css_style_sheet_code;
      if self.es_module() {
        code.push_str(&format!("{ns_obj}({module_argument}.exports = {{}});\n"));
        code.push_str(&format!(
          "{module_argument}.exports.default = __css_style_sheet;\n"
        ));
      } else {
        code.push_str(&format!("{module_argument}.exports = __css_style_sheet;\n"));
      }
      code.push_str(&self.render_accept_hmr());
      code
    };

    source
  }

  fn generate_css_text_exports(&mut self, css_js_string: &str) -> String {
    let module_argument = self
      .generate_context
      .runtime_template
      .render_module_argument(ModuleArgument::Module);
    let (ns_obj, left, right) = self.get_namespace_object_parts();

    let source = if let Some((decl_name, exports_string)) = self.stringified_used_css_exports() {
      let hmr_code = self.render_exports_hmr(&decl_name);
      let mut code = String::new();
      code.push_str(&exports_string);
      code.push('\n');
      code.push_str(&hmr_code);
      code.push('\n');
      code.push_str(&ns_obj);
      code.push_str(&left);
      code.push_str(&module_argument);
      code.push_str(".exports = Object.assign({}, ");
      code.push_str(&decl_name);
      code.push(')');
      code.push_str(&right);
      code.push_str(";\n");
      code.push_str(&module_argument);
      code.push_str(".exports.default = ");
      code.push_str(css_js_string);
      code.push_str(";\n");
      code
    } else {
      let mut code = String::new();
      if self.es_module() {
        code.push_str(&format!("{ns_obj}({module_argument}.exports = {{}});\n"));
        code.push_str(&module_argument);
        code.push_str(".exports.default = ");
        code.push_str(css_js_string);
        code.push_str(";\n");
      } else {
        code.push_str(&module_argument);
        code.push_str(".exports = ");
        code.push_str(css_js_string);
        code.push_str(";\n");
      }
      code.push_str(&self.render_accept_hmr());
      code
    };

    source
  }

  fn stringified_used_css_exports(&mut self) -> Option<(&'static str, String)> {
    let build_info = self.module.build_info();
    let exports = build_info.css_exports.as_ref()?;

    if let Some(local_names) = &build_info.css_local_names {
      let unused_exports = get_unused_local_ident(
        exports,
        local_names,
        self.module.identifier(),
        self.generate_context.runtime,
        &self.generate_context.compilation.exports_info_artifact,
      );
      self.generate_context.data.insert(unused_exports);
    }

    let exports = get_used_exports(
      exports,
      self.module.identifier(),
      self.generate_context.runtime,
      &self.generate_context.compilation.exports_info_artifact,
    );

    let (decl_name, exports_string) = self.stringified_exports(exports);

    Some((decl_name, exports_string))
  }

  fn get_namespace_object_parts(&mut self) -> (String, String, String) {
    if self.es_module() {
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

  fn render_exports_hmr(&self, decl_name: &str) -> String {
    let with_hmr = self.with_hmr;
    let module_argument = self.module_argument();

    if with_hmr {
      format!(
        "// only invalidate when locals change\n\
         var stringified_exports = JSON.stringify({decl_name});\n\
         if ({module_argument}.hot.data && {module_argument}.hot.data.exports && {module_argument}.hot.data.exports != stringified_exports) {{\n\
         {module_argument}.hot.invalidate();\n\
         }} else {{\n\
         {module_argument}.hot.accept();\n\
         }}\n\
         {module_argument}.hot.dispose(function(data) {{ data.exports = stringified_exports; }});"
      )
    } else {
      String::new()
    }
  }

  fn render_accept_hmr(&self) -> String {
    let module_argument = self.module_argument();
    let with_hmr = self.with_hmr;

    if with_hmr {
      format!("{module_argument}.hot.accept();\n")
    } else {
      String::new()
    }
  }

  fn stringified_exports<'b>(&mut self, exports: CssExportsRef<'b>) -> (&'static str, String) {
    let compilation = self.generate_context.compilation;
    let module = self.module;
    let runtime = self.generate_context.runtime;

    let mut stringified_exports = String::new();
    let exports_info = compilation
      .exports_info_artifact
      .get_exports_info_data(&module.identifier());
    for (key, elements) in exports {
      let export_info = exports_info.get_read_only_export_info(&Atom::from(key));
      let used_name = match export_info.get_used_name(None, runtime) {
        Some(UsedNameItem::Str(name)) => name.to_string(),
        _ => key.to_string(),
      };
      let content = self.render_css_export_content(elements, true);
      stringified_exports.push_str(&format!(
        "  {}: {},",
        json_stringify_str(&used_name),
        content,
      ));
      stringified_exports.push_str("\n");
    }

    let decl_name = "exports";
    (
      decl_name,
      format!("var {decl_name} = {{\n{stringified_exports}}};"),
    )
  }

  fn css_modules_exports_to_string(
    &mut self,
    exports: CssExportsRef,
    ns_obj: &str,
    left: &str,
    right: &str,
  ) -> String {
    let (decl_name, exports_string) = self.stringified_exports(exports);
    let module_argument = self
      .generate_context
      .runtime_template
      .render_module_argument(ModuleArgument::Module);
    let hmr_code = self.render_exports_hmr(decl_name);
    let mut code = format!(
      "{exports_string}\n{hmr_code}\n{ns_obj}{left}{module_argument}.exports = {decl_name}"
    );
    code.push_str(right);
    code.push_str(";\n");
    code
  }

  fn css_modules_exports_to_concatenate_module_string(&mut self, exports: CssExportsRef) {
    if self.generate_context.concatenation_scope.is_none() {
      return;
    }
    let module = self.module;

    let mut used_identifiers: HashSet<Cow<'_, str>> = HashSet::default();
    let exports_info = self
      .generate_context
      .compilation
      .exports_info_artifact
      .get_exports_info_data(&module.identifier());
    for (key, elements) in exports {
      let export_info = exports_info.get_read_only_export_info(&Atom::from(key));
      let used_name = match export_info.get_used_name(None, self.generate_context.runtime) {
        Some(UsedNameItem::Str(name)) => name.to_string(),
        _ => key.to_string(),
      };
      let content = self.render_css_export_content(elements, false);
      let mut identifier: Cow<'_, str> = Cow::Owned(to_identifier(&used_name).into_owned());
      if RESERVED_IDENTIFIER.contains(identifier.as_ref()) {
        identifier = Cow::Owned(format!("_{identifier}"));
      }
      let mut i = 0;
      while used_identifiers.contains(&identifier) {
        let mut i_buffer = itoa::Buffer::new();
        let i_str = i_buffer.format(i);
        identifier = Cow::Owned(format!("{identifier}{i_str}"));
        i += 1;
      }
      self.concat_source.add(RawStringSource::from(format!(
        "var {identifier} = {content};\n"
      )));
      used_identifiers.insert(identifier.clone());
      let Some(ref mut scope) = self.generate_context.concatenation_scope else {
        unreachable!();
      };
      scope.register_export(key.into(), identifier.into_owned());
    }
  }

  fn render_css_export_content(
    &mut self,
    elements: &FxIndexSet<CssExport>,
    unescape_referenced_ident: bool,
  ) -> String {
    let compilation = self.generate_context.compilation;
    let module = self.module;
    let runtime = self.generate_context.runtime;

    let module_graph = compilation.get_module_graph();
    elements
      .iter()
      .map(
        |CssExport {
           ident,
           from,
           id: _,
           orig_name: _,
         }| match from {
          None => json_stringify_str(ident),
          Some(from_name) => {
            let from = module
              .get_dependencies()
              .iter()
              .find_map(|id| {
                let dependency = module_graph.dependency_by_id(id);
                let request = if let Some(d) = dependency.as_module_dependency() {
                  Some(d.request())
                } else {
                  dependency.as_context_dependency().map(|d| d.request())
                };
                if let Some(request) = request {
                  if request == from_name {
                    return module_graph.module_graph_module_by_dependency_id(id);
                  }
                }
                None
              })
              .expect("should have css from module");

            let from_exports_info = compilation
              .exports_info_artifact
              .get_exports_info_data(&from.module_identifier);
            let from_used_name = match from_exports_info
              .get_read_only_export_info(&Atom::from(ident.as_str()))
              .get_used_name(None, runtime)
            {
              Some(UsedNameItem::Str(name)) => {
                let name = if unescape_referenced_ident {
                  Cow::Owned(unescape(name.as_str()).into_owned())
                } else {
                  Cow::Borrowed(name.as_str())
                };
                json_stringify_str(name.as_ref())
              }
              _ => {
                let ident = if unescape_referenced_ident {
                  unescape(ident)
                } else {
                  Cow::Borrowed(ident.as_str())
                };
                json_stringify_str(ident.as_ref())
              }
            };

            let from = json_stringify(
              ChunkGraph::get_module_id(&compilation.module_ids_artifact, from.module_identifier)
                .expect("should have module"),
            );
            format!(
              "{}({from})[{}]",
              self
                .generate_context
                .runtime_template
                .render_runtime_globals(&RuntimeGlobals::REQUIRE),
              from_used_name
            )
          }
        },
      )
      .collect::<Vec<_>>()
      .join(" + \" \" + ")
  }
}
