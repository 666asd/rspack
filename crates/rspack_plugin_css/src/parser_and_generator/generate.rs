use std::{borrow::Cow, collections::HashSet, fmt::format};

use rspack_core::{
  ChunkGraph, CompilerOptions, CssExport, CssExportType, CssExports, Dependency, GenerateContext,
  InitFragmentRenderContext, Module, ModuleArgument, ModuleInitFragments, RESERVED_IDENTIFIER,
  ResourceData, RuntimeGlobals, TemplateContext, UsageState, UsedNameItem,
  rspack_sources::{BoxSource, ConcatSource, RawStringSource, ReplaceSource, SourceExt},
  to_identifier,
};
use rspack_error::Result;
use rspack_util::{atom::Atom, fx_hash::FxIndexSet, itoa, json_stringify, json_stringify_str};
use rustc_hash::FxHashMap;

use crate::{
  dependency::{
    CssLocalIdentDependency, CssSelfReferenceLocalIdentDependency,
    CssSelfReferenceLocalIdentReplacement,
  },
  parser_and_generator::{
    CssExportsRef, CssParserAndGenerator, get_unused_local_ident, get_used_exports,
  },
  utils::{LocalIdentOptions, export_locals_convention, unescape},
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

pub(crate) struct CssGenerator<'a> {
  source: &'a BoxSource,
  module: &'a dyn Module,
  generate_context: &'a mut GenerateContext<'a>,
  with_hmr: bool,
  export_type: Option<CssExportType>,
  es_module: bool,
  exports_only: bool,
  module_argument: String,
  concat_source: ConcatSource,
}

impl<'a> CssGenerator<'a> {
  pub fn new(
    source: &'a BoxSource,
    module: &'a dyn Module,
    generate_context: &'a mut GenerateContext<'a>,
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

  pub fn generate_javascript_source(&mut self) -> BoxSource {
    let concat_source = &mut self.concat_source;
    let export_type = self.export_type.clone();
    let exports_only = self.exports_only;

    match export_type {
      Some(CssExportType::Style) if exports_only => self.generate_js_exports(),
      Some(CssExportType::Style) => {
        let css_js_string = self.stringify_css_source_for_javascript();

        concat_source.add(RawStringSource::from(
          self.render_css_inject_style(&css_js_string),
        ));
        concat_source.add(self.generate_js_exports());

        concat_source.boxed()
      }
      Some(CssExportType::CssStyleSheet) => {
        let css_js_string = self.stringify_css_source_for_javascript();
        self.generate_css_style_sheet_exports(&css_js_string)
      }
      Some(CssExportType::Text) => {
        let css_js_string = self.stringify_css_source_for_javascript();
        self.generate_css_text_exports(&css_js_string)
      }
      _ => self.generate_js_exports(),
    }
  }

  fn generate_js_exports(&mut self) -> BoxSource {
    let module = self.module;
    let generate_context = self.generate_context;
    let with_hmr = self.with_hmr;

    let build_info = module.build_info();
    if generate_context.concatenation_scope.is_some() {
      let mut concate_source = ConcatSource::default();
      if let Some(ref exports) = build_info.css_exports {
        let exports_info_artifact = &generate_context.compilation.exports_info_artifact;
        if let Some(local_names) = &build_info.css_local_names {
          let unused_exports = get_unused_local_ident(
            exports,
            local_names,
            module.identifier(),
            generate_context.runtime,
            exports_info_artifact,
          );
          generate_context.data.insert(unused_exports);
        }
        let exports = get_used_exports(
          exports,
          module.identifier(),
          generate_context.runtime,
          exports_info_artifact,
        );

        self.css_modules_exports_to_concatenate_module_string(exports);
      }
      concate_source.boxed()
    } else {
      let exports_info = generate_context
        .compilation
        .exports_info_artifact
        .get_exports_info_data(&module.identifier());
      let (ns_obj, left, right) = if self.es_module()
        && exports_info
          .other_exports_info()
          .get_used(generate_context.runtime)
          != UsageState::Unused
      {
        (
          generate_context
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
            generate_context.runtime,
            &generate_context.compilation.exports_info_artifact,
          );
          generate_context.data.insert(unused_exports);
        }

        let exports = get_used_exports(
          exports,
          module.identifier(),
          generate_context.runtime,
          &generate_context.compilation.exports_info_artifact,
        );

        self.css_modules_exports_to_string(exports, &ns_obj, &left, &right)
      } else {
        let module_argument = generate_context
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
      RawStringSource::from(exports_str).boxed()
    }
  }

  fn stringify_css_source_for_javascript(&self) -> String {
    let generate_context = self.generate_context;
    let module = self.module;

    let mut source = ReplaceSource::new(self.source.clone());
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

    generate_context.concatenation_scope = context.concatenation_scope.take();

    let css_source = source.boxed();
    let css_text = css_source
      .source()
      .into_string_lossy()
      .replace(crate::utils::AUTO_PUBLIC_PATH_PLACEHOLDER, "");

    json_stringify_str(&css_text)
  }

  fn render_css_inject_style(&self, css_js_string: &str) -> String {
    let generate_context = self.generate_context;
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

    format!(
      "{}({}, {});\n",
      generate_context
        .runtime_template
        .render_runtime_globals(&RuntimeGlobals::CSS_INJECT_STYLE),
      json_stringify_str(&module_id),
      css_js_string
    )
  }

  fn generate_css_style_sheet_exports(&self, css_js_string: &str) -> BoxSource {
    let generate_context = self.generate_context;
    let module = self.module;

    generate_context
      .runtime_template
      .runtime_requirements_mut()
      .insert(RuntimeGlobals::CSS_STYLE_SHEET);

    let module_argument = generate_context
      .runtime_template
      .render_module_argument(ModuleArgument::Module);
    let (ns_obj, left, right) = self.get_namespace_object_parts(generate_context);
    let css_style_sheet_code = format!(
      "var __css_style_sheet = {}({});\n",
      generate_context
        .runtime_template
        .render_runtime_globals(&RuntimeGlobals::CSS_STYLE_SHEET),
      css_js_string
    );

    let source = if let Some((decl_name, exports_string)) =
      self.stringified_used_css_exports(module, generate_context)
    {
      let hmr_code = self.render_exports_hmr(&decl_name);
      let mut code = format!(
        "{css_style_sheet_code}{exports_string}\n{hmr_code}\n{ns_obj}{left}{module_argument}.exports = Object.assign(__css_style_sheet, {decl_name})",
      );
      code.push_str(&right);
      code.push_str(";\n");
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

    RawStringSource::from(source).boxed()
  }

  fn generate_css_text_exports(&self, css_js_string: &str) -> BoxSource {
    let module_argument = self.module_argument;
    let generate_context = self.generate_context;
    let with_hmr = self.with_hmr;
    let module = self.module;

    let module_argument = generate_context
      .runtime_template
      .render_module_argument(ModuleArgument::Module);
    let (ns_obj, left, right) = self.get_namespace_object_parts(generate_context);

    let source = if let Some((decl_name, exports_string)) =
      self.stringified_used_css_exports(module, generate_context)
    {
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
      code.push_str(".default = ");
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

    RawStringSource::from(source).boxed()
  }

  fn stringified_used_css_exports(
    &self,
    module: &dyn Module,
    generate_context: &mut GenerateContext,
  ) -> Option<(&'static str, String)> {
    let build_info = module.build_info();
    let exports = &build_info.css_exports?;

    if let Some(local_names) = &build_info.css_local_names {
      let unused_exports = get_unused_local_ident(
        exports,
        local_names,
        module.identifier(),
        generate_context.runtime,
        &generate_context.compilation.exports_info_artifact,
      );
      generate_context.data.insert(unused_exports);
    }

    let exports = get_used_exports(
      exports,
      module.identifier(),
      generate_context.runtime,
      &generate_context.compilation.exports_info_artifact,
    );

    let (decl_name, exports_string) = self.stringified_exports(exports);

    Some((decl_name, exports_string))
  }

  fn get_namespace_object_parts(
    &self,
    generate_context: &mut GenerateContext,
  ) -> (String, String, String) {
    if self.es_module() {
      (
        generate_context
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

  fn stringified_exports<'b>(&self, exports: CssExportsRef<'b>) -> (&'static str, String) {
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
    &self,
    exports: CssExportsRef,
    ns_obj: &str,
    left: &str,
    right: &str,
  ) -> String {
    let runtime_template = self.generate_context.runtime_template;

    let (decl_name, exports_string) = self.stringified_exports(exports);
    let module_argument = runtime_template.render_module_argument(ModuleArgument::Module);
    let hmr_code = self.render_exports_hmr(decl_name);
    let mut code = format!(
      "{exports_string}\n{hmr_code}\n{ns_obj}{left}{module_argument}.exports = {decl_name}"
    );
    code.push_str(right);
    code.push_str(";\n");
    code
  }

  fn css_modules_exports_to_concatenate_module_string(&mut self, exports: CssExportsRef) {
    let generate_context = self.generate_context;
    let module = self.module;
    let concat_source = &mut self.concat_source;

    let GenerateContext {
      compilation,
      concatenation_scope,
      runtime,
      ..
    } = generate_context;
    let Some(scope) = concatenation_scope else {
      return;
    };
    let mut used_identifiers = HashSet::default();
    let exports_info = compilation
      .exports_info_artifact
      .get_exports_info_data(&module.identifier());
    for (key, elements) in exports {
      let export_info = exports_info.get_read_only_export_info(&Atom::from(key));
      let used_name = match export_info.get_used_name(None, *runtime) {
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
      concat_source.add(RawStringSource::from(format!(
        "var {identifier} = {content};\n"
      )));
      used_identifiers.insert(identifier.clone());
      scope.register_export(key.into(), identifier.into_owned());
    }
  }

  fn render_css_export_content(
    &self,
    elements: &FxIndexSet<CssExport>,
    unescape_referenced_ident: bool,
  ) -> String {
    let compilation = self.generate_context.compilation;
    let module = self.module;
    let runtime_template = self.generate_context.runtime_template;
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
              runtime_template.render_runtime_globals(&RuntimeGlobals::REQUIRE),
              from_used_name
            )
          }
        },
      )
      .collect::<Vec<_>>()
      .join(" + \" \" + ")
  }
}
