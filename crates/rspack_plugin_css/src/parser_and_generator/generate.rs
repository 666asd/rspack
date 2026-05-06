use rspack_core::{
  ChunkGraph, CompilerOptions, CssExport, CssExportType, CssExports, Dependency, GenerateContext,
  Module, ModuleArgument, ModuleInitFragments, ResourceData, RuntimeGlobals, TemplateContext,
  UsageState,
  rspack_sources::{BoxSource, ConcatSource, RawStringSource, ReplaceSource, SourceExt},
};
use rspack_error::Result;
use rspack_util::fx_hash::FxIndexSet;
use rustc_hash::FxHashMap;

use crate::{
  dependency::{
    CssLocalIdentDependency, CssSelfReferenceLocalIdentDependency,
    CssSelfReferenceLocalIdentReplacement,
  },
  parser_and_generator::{CssParserAndGenerator, get_unused_local_ident, get_used_exports},
  utils::{
    LocalIdentOptions, css_modules_exports_to_concatenate_module_string,
    css_modules_exports_to_string, export_locals_convention, unescape,
  },
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

impl CssParserAndGenerator {
  pub fn generate_javascript_source(
    &self,
    source: &BoxSource,
    module: &dyn Module,
    generate_context: &mut GenerateContext,
  ) -> Result<BoxSource> {
    let with_hmr = self.hot;

    match self.export_type() {
      Some(CssExportType::Style) if self.exports_only() => {
        self.generate_js_exports(module, generate_context, with_hmr)
      }
      Some(CssExportType::Style) => {
        let mut concat_source = ConcatSource::default();
        let css_js_string =
          self.stringify_css_source_for_javascript(source, module, generate_context)?;

        concat_source.add(RawStringSource::from(self.render_css_inject_style(
          module,
          generate_context,
          &css_js_string,
        )?));
        concat_source.add(self.generate_js_exports(module, generate_context, with_hmr)?);

        Ok(concat_source.boxed())
      }
      Some(CssExportType::CssStyleSheet) => {
        let css_js_string =
          self.stringify_css_source_for_javascript(source, module, generate_context)?;
        self.generate_css_style_sheet_exports(module, generate_context, &css_js_string, with_hmr)
      }
      Some(CssExportType::Text) => {
        let css_js_string =
          self.stringify_css_source_for_javascript(source, module, generate_context)?;
        self.generate_css_text_exports(module, generate_context, &css_js_string, with_hmr)
      }
      _ => self.generate_js_exports(module, generate_context, with_hmr),
    }
  }

  pub async fn handle_local_ident_usage(
    &self,
    name: &str,
    range: css_module_lexer::Range,
    resource_data: &ResourceData,
    compiler_options: &CompilerOptions,
    css_exports: &mut Option<CssExports>,
    dependencies: &mut Vec<Box<dyn Dependency>>,
  ) -> Result<()> {
    let name = unescape(name);
    let (local_ident, convention_names) = self
      .resolve_local_ident_and_update_exports(resource_data, compiler_options, &name, css_exports)
      .await?;
    dependencies.push(Box::new(CssSelfReferenceLocalIdentDependency::new(
      convention_names,
      vec![CssSelfReferenceLocalIdentReplacement {
        local_ident,
        range: (range.start, range.end).into(),
      }],
    )));
    Ok(())
  }

  pub async fn handle_local_ident_declaration(
    &self,
    name: &str,
    range: css_module_lexer::Range,
    resource_data: &ResourceData,
    compiler_options: &CompilerOptions,
    css_exports: &mut Option<CssExports>,
    css_local_names: &mut Option<FxHashMap<String, String>>,
    dependencies: &mut Vec<Box<dyn Dependency>>,
  ) -> Result<()> {
    let name = unescape(name);
    let (local_ident, convention_names) = self
      .resolve_local_ident_and_update_exports(resource_data, compiler_options, &name, css_exports)
      .await?;

    let local_names = css_local_names.get_or_insert_default();
    local_names.insert(name.into_owned(), local_ident.clone());

    dependencies.push(Box::new(CssLocalIdentDependency::new(
      local_ident,
      convention_names,
      range.start,
      range.end,
    )));
    Ok(())
  }

  pub async fn resolve_local_ident_and_update_exports(
    &self,
    resource_data: &ResourceData,
    compiler_options: &CompilerOptions,
    name: &str,
    css_exports: &mut Option<CssExports>,
  ) -> Result<(String, Vec<String>)> {
    let local_ident_hash_digest = self
      .generator_options
      .local_ident_hash_digest
      .as_deref()
      .map(Into::into);
    let local_ident_hash_digest_length = self
      .generator_options
      .local_ident_hash_digest_length
      .map(|len| len as usize);
    let local_ident_hash_function = self
      .generator_options
      .local_ident_hash_function
      .as_deref()
      .map(Into::into);
    let local_ident_hash_salt = self
      .generator_options
      .local_ident_hash_salt
      .clone()
      .map(Some)
      .map(Into::into);

    let local_ident = LocalIdentOptions::new(
      resource_data,
      self.local_ident_name(),
      compiler_options,
      local_ident_hash_digest.as_ref(),
      local_ident_hash_digest_length,
      local_ident_hash_function.as_ref(),
      local_ident_hash_salt.as_ref(),
    )
    .get_local_ident(name)
    .await?;
    let convention = self.convention();
    let exports = css_exports.get_or_insert_default();
    let convention_names = export_locals_convention(name, convention);
    for convention_name in convention_names.iter() {
      update_css_exports(
        exports,
        convention_name.to_owned(),
        CssExport {
          ident: local_ident.clone(),
          orig_name: name.to_owned(),
          from: None,
          id: None,
        },
      );
    }
    Ok((local_ident, convention_names))
  }

  fn generate_js_exports(
    &self,
    module: &dyn Module,
    generate_context: &mut GenerateContext,
    with_hmr: bool,
  ) -> Result<BoxSource> {
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

        css_modules_exports_to_concatenate_module_string(
          exports,
          module,
          generate_context,
          &mut concate_source,
        )?;
      }
      Ok(concate_source.boxed())
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

        css_modules_exports_to_string(
          exports,
          module,
          generate_context.compilation,
          generate_context.runtime,
          generate_context.runtime_template,
          &ns_obj,
          &left,
          &right,
          with_hmr,
        )?
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
      Ok(RawStringSource::from(exports_str).boxed())
    }
  }

  fn stringify_css_source_for_javascript(
    &self,
    source: &BoxSource,
    module: &dyn Module,
    generate_context: &mut GenerateContext,
  ) -> Result<String> {
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

    generate_context.concatenation_scope = context.concatenation_scope.take();

    let css_source = source.boxed();
    let css_text = css_source
      .source()
      .into_string_lossy()
      .replace(crate::utils::AUTO_PUBLIC_PATH_PLACEHOLDER, "");

    serde_json::to_string(&css_text).map_err(|e| rspack_error::error!("{}", e))
  }

  fn render_css_inject_style(
    &self,
    module: &dyn Module,
    generate_context: &mut GenerateContext,
    css_js_string: &str,
  ) -> Result<String> {
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

    Ok(format!(
      "{}({}, {});\n",
      generate_context
        .runtime_template
        .render_runtime_globals(&RuntimeGlobals::CSS_INJECT_STYLE),
      serde_json::to_string(&module_id).map_err(|e| rspack_error::error!("{}", e))?,
      css_js_string
    ))
  }

  fn generate_css_style_sheet_exports(
    &self,
    module: &dyn Module,
    generate_context: &mut GenerateContext,
    css_js_string: &str,
    with_hmr: bool,
  ) -> Result<BoxSource> {
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
      self.stringified_used_css_exports(module, generate_context)?
    {
      let hmr_code = Self::render_exports_hmr(&module_argument, &decl_name, with_hmr);
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
      code.push_str(&Self::render_accept_hmr(&module_argument, with_hmr));
      code
    };

    Ok(RawStringSource::from(source).boxed())
  }

  fn generate_css_text_exports(
    &self,
    module: &dyn Module,
    generate_context: &mut GenerateContext,
    css_js_string: &str,
    with_hmr: bool,
  ) -> Result<BoxSource> {
    let module_argument = generate_context
      .runtime_template
      .render_module_argument(ModuleArgument::Module);
    let (ns_obj, left, right) = self.get_namespace_object_parts(generate_context);

    let source = if let Some((decl_name, exports_string)) =
      self.stringified_used_css_exports(module, generate_context)?
    {
      let hmr_code = Self::render_exports_hmr(&module_argument, &decl_name, with_hmr);
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
      code.push_str(&Self::render_accept_hmr(&module_argument, with_hmr));
      code
    };

    Ok(RawStringSource::from(source).boxed())
  }

  fn stringified_used_css_exports(
    &self,
    module: &dyn Module,
    generate_context: &mut GenerateContext,
  ) -> Result<Option<(&'static str, String)>> {
    let build_info = module.build_info();
    let Some(exports) = &build_info.css_exports else {
      return Ok(None);
    };

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

    let (decl_name, exports_string) = crate::utils::stringified_exports(
      exports,
      generate_context.compilation,
      generate_context.runtime_template,
      module,
      generate_context.runtime,
    )?;

    Ok(Some((decl_name, exports_string)))
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

  fn render_exports_hmr(module_argument: &str, decl_name: &str, with_hmr: bool) -> String {
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

  fn render_accept_hmr(module_argument: &str, with_hmr: bool) -> String {
    if with_hmr {
      format!("{module_argument}.hot.accept();\n")
    } else {
      String::new()
    }
  }
}
