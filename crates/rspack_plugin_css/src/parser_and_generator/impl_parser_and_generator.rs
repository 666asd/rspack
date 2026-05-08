use std::{borrow::Cow, sync::Arc};

use once_cell::sync::OnceCell;
use rspack_cacheable::cacheable_dyn;
use rspack_core::{
  BoxDependencyTemplate, BoxModuleDependency, BuildMetaDefaultObject, BuildMetaExportsType,
  ChunkGraph, Compilation, ConstDependency, CssExportType, CssParserImport, CssParserImportContext,
  Dependency, DependencyRange, DependencyType, GenerateContext, Module, ModuleGraph,
  ModuleInitFragments, ModuleType, NormalModule, ParseContext, ParseResult, ParserAndGenerator,
  RuntimeGlobals, RuntimeSpec, SourceType, StaticExportsDependency, StaticExportsSpec,
  TemplateContext,
  diagnostics::map_box_diagnostics_to_module_parse_diagnostics,
  remove_bom,
  rspack_sources::{BoxSource, ReplaceSource, Source, SourceExt},
};
pub use rspack_core::{CssExport, CssExports};
use rspack_error::{Diagnostic, IntoTWithDiagnosticArray, Result, Severity, TWithDiagnosticArray};
use rspack_hash::{RspackHash, RspackHashDigest};
use rspack_util::ext::DynHash;
use rustc_hash::FxHashMap;

use crate::{
  dependency::{
    CssComposeDependency, CssExportDependency, CssIcssImportDependency, CssImportDependency,
    CssImportMode, CssLayer, CssLocalIdentDependency, CssMedia,
    CssSelfReferenceLocalIdentDependency, CssSupports, CssUrlDependency,
  },
  parser_and_generator::{
    generator::{CssModuleGenerator, update_css_exports},
    *,
  },
  utils::{
    css_parsing_traceable_error, export_locals_convention, normalize_url,
    replace_module_request_prefix, unescape,
  },
};

#[derive(Debug, Clone)]
struct IcssImportReference {
  request: String,
  import_name: String,
}

fn is_css_identifier_char(c: char) -> bool {
  c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '\\')
}

fn range_contains(ranges: &[css_module_lexer::Range], pos: u32) -> bool {
  ranges
    .iter()
    .any(|range| range.start <= pos && pos < range.end)
}

fn next_non_whitespace_char(input: &str, mut pos: usize) -> Option<char> {
  while let Some(c) = input[pos..].chars().next() {
    if c.is_whitespace() {
      pos += c.len_utf8();
      continue;
    }
    if c == '/' && input[pos..].starts_with("/*") {
      let comment = &input[pos + 2..];
      let end = comment.find("*/")?;
      pos += end + 4;
      continue;
    }
    return Some(c);
  }
  None
}

fn collect_icss_import_usage_ranges(
  source: &str,
  aliases: &FxHashMap<String, IcssImportReference>,
  removed_ranges: &[css_module_lexer::Range],
) -> Vec<(IcssImportReference, DependencyRange)> {
  let mut replacements = Vec::new();

  for (alias, reference) in aliases {
    let mut offset = 0;

    while let Some(found) = source[offset..].find(alias) {
      let start = offset + found;
      let end = start + alias.len();
      offset = end;

      let start_pos = start as u32;
      if range_contains(removed_ranges, start_pos) {
        continue;
      }

      if source[..start]
        .chars()
        .next_back()
        .is_some_and(|c| is_css_identifier_char(c) || matches!(c, '.' | '#'))
      {
        continue;
      }

      if source[end..]
        .chars()
        .next()
        .is_some_and(is_css_identifier_char)
      {
        continue;
      }

      if next_non_whitespace_char(source, end).is_some_and(|c| c == ':') {
        continue;
      }

      replacements.push((
        reference.clone(),
        DependencyRange::new(start as u32, end as u32),
      ));
    }
  }

  replacements
}

#[cacheable_dyn]
#[async_trait::async_trait]
impl ParserAndGenerator for CssParserAndGenerator {
  fn source_types(&self, module: &dyn Module, module_graph: &ModuleGraph) -> &[SourceType] {
    // 如果 export_type 是 style、css_style_sheet 或 text，我们只返回 JavaScript
    if matches!(
      self.export_type(),
      Some(CssExportType::Style) | Some(CssExportType::CssStyleSheet) | Some(CssExportType::Text)
    ) {
      return CSS_MODULE_EXPORTS_ONLY_SOURCE_TYPE_LIST;
    }

    if self.exports_only() {
      return CSS_MODULE_EXPORTS_ONLY_SOURCE_TYPE_LIST;
    }

    let no_need_js = module_graph
      .get_incoming_connections(&module.identifier())
      .all(|conn| {
        let dep = module_graph.dependency_by_id(&conn.dependency_id);
        matches!(
          dep.dependency_type(),
          DependencyType::CssImport | DependencyType::CssIcssImport | DependencyType::EsmImport
        )
      });

    if no_need_js {
      CSS_MODULE_SOURCE_TYPE_LIST
    } else {
      CSS_MODULE_AND_JS_SOURCE_TYPE_LIST
    }
  }

  fn size(&self, module: &dyn Module, source_type: Option<&SourceType>) -> f64 {
    match source_type.unwrap_or(&SourceType::Css) {
      SourceType::JavaScript => 42.0,
      SourceType::Css => module.source().map_or(0, |source| source.size()) as f64,
      _ => unreachable!(),
    }
  }

  async fn parse<'a>(
    &mut self,
    parse_context: ParseContext<'a>,
  ) -> Result<TWithDiagnosticArray<ParseResult>> {
    let ParseContext {
      source,
      module_type,
      resource_data,
      compiler_options,
      build_info,
      build_meta,
      loaders,
      module_match_resource,
      ..
    } = parse_context;

    build_info.strict = true;
    build_meta.exports_type = if self.named_exports() {
      BuildMetaExportsType::Namespace
    } else {
      BuildMetaExportsType::Default
    };
    build_meta.default_object = if self.named_exports() {
      BuildMetaDefaultObject::False
    } else {
      BuildMetaDefaultObject::Redirect
    };

    let source = remove_bom(source);
    let source_code = source.source().into_string_lossy();
    let resource_data = module_match_resource.unwrap_or(resource_data);
    let resource_path = resource_data.path();
    let cached_source_code = OnceCell::new();
    let get_source_code = || {
      let s = cached_source_code.get_or_init(|| Arc::new(source_code.to_string()));
      s.clone()
    };

    let mode = match module_type {
      ModuleType::CssModule => css_module_lexer::Mode::Local,
      ModuleType::CssAuto
        if resource_path.is_some()
          && REGEX_IS_MODULES.is_match(
            resource_path
              .as_ref()
              .expect("should have resource_path for module_type css/auto")
              .as_str(),
          ) =>
      {
        css_module_lexer::Mode::Local
      }
      _ => css_module_lexer::Mode::Css,
    };

    let import_mode = match mode {
      css_module_lexer::Mode::Local => Some(CssImportMode::Local),
      css_module_lexer::Mode::Global => Some(CssImportMode::Global),
      css_module_lexer::Mode::Css | css_module_lexer::Mode::Pure => None,
    };

    let mut diagnostics: Vec<Diagnostic> = vec![];
    let mut dependencies: Vec<Box<dyn Dependency>> = vec![];
    let mut presentational_dependencies: Vec<BoxDependencyTemplate> = vec![];
    let mut code_generation_dependencies: Vec<BoxModuleDependency> = vec![];
    let mut css_exports: Option<CssExports> = None;
    let mut css_local_names: Option<FxHashMap<String, String>> = None;
    let mut icss_import_request: Option<String> = None;
    let mut icss_imports: FxHashMap<String, IcssImportReference> = Default::default();
    let mut icss_removed_ranges = Vec::new();

    let (deps, warnings) = css_module_lexer::collect_dependencies(&source_code, mode);

    let animation_enabled = self.animation().unwrap_or(true);
    let container_enabled = self.container().unwrap_or(true);
    let custom_idents_enabled = self.custom_idents().unwrap_or(true);
    let dashed_idents_enabled = self.dashed_idents().unwrap_or(true);
    let function_enabled = self.function().unwrap_or(true);
    let grid_enabled = self.grid().unwrap_or(true);

    for dependency in deps {
      match dependency {
        css_module_lexer::Dependency::Url {
          request,
          range,
          kind,
        } => {
          if request.trim().is_empty() {
            continue;
          }
          if !self.url() {
            continue;
          }

          let request = replace_module_request_prefix(
            request,
            &mut diagnostics,
            get_source_code,
            range.start,
            range.end,
          );
          let request = normalize_url(request);
          let dep = Box::new(CssUrlDependency::new(
            request,
            DependencyRange::new(range.start, range.end),
            matches!(kind, css_module_lexer::UrlRangeKind::Function),
          ));
          dependencies.push(dep.clone());
          code_generation_dependencies.push(dep);
        }
        css_module_lexer::Dependency::Import {
          request,
          range,
          media,
          supports,
          layer,
        } => {
          if request.is_empty() {
            presentational_dependencies.push(Box::new(ConstDependency::new(
              (range.start, range.end).into(),
              "".into(),
            )));
            continue;
          }
          // Check the import option
          let should_import = match self.resolve_import() {
            CssParserImport::Bool(b) => *b,
            CssParserImport::Func(f) => {
              // Call the filter function with the import arguments
              let args = CssParserImportContext {
                url: request.to_string(),
                media: media.map(|s| s.to_string()),
                resource_path: resource_path
                  .map(|p| p.as_str().to_string())
                  .unwrap_or_default(),
                supports: supports.map(|s| s.to_string()),
                layer: layer.map(|s| s.to_string()),
              };
              (f(args).await).unwrap_or(true)
            }
          };
          if !should_import {
            continue;
          }
          let request = replace_module_request_prefix(
            request,
            &mut diagnostics,
            get_source_code,
            range.start,
            range.end,
          );
          dependencies.push(Box::new(CssImportDependency::new(
            request.to_string(),
            DependencyRange::new(range.start, range.end),
            media.map(|s| s.to_string()),
            supports.map(|s| s.to_string()),
            layer.map(|s| {
              if s.is_empty() {
                CssLayer::Anonymous
              } else {
                CssLayer::Named(s.to_string())
              }
            }),
            import_mode,
          )));
        }
        css_module_lexer::Dependency::Replace { content, range } => presentational_dependencies
          .push({
            let original = source_code
              .get(range.start as usize..range.end as usize)
              .unwrap_or_default();
            if original.starts_with(":import(") || original.starts_with(":export") {
              icss_removed_ranges.push(range.clone());
            }
            Box::new(ConstDependency::new(
              (range.start, range.end).into(),
              content.into(),
            ))
          }),
        css_module_lexer::Dependency::LocalClass { name, range, .. }
        | css_module_lexer::Dependency::LocalId { name, range, .. } => {
          let (_prefix, name) = name.split_at(1); // split '#' or '.'
          let name = unescape(name);

          let (local_ident, convention_names) = self
            .resolve_local_ident_and_update_exports(
              resource_data,
              compiler_options,
              &name,
              &mut css_exports,
            )
            .await?;

          let local_names = css_local_names.get_or_insert_default();
          local_names.insert(name.into_owned(), local_ident.clone());

          dependencies.push(Box::new(CssLocalIdentDependency::new(
            local_ident,
            convention_names,
            range.start + 1,
            range.end,
          )));
        }
        css_module_lexer::Dependency::LocalKeyframes { name, range, .. } => {
          if !animation_enabled {
            continue;
          }
          self
            .handle_local_ident_usage(
              name,
              range,
              resource_data,
              compiler_options,
              &mut css_exports,
              &mut dependencies,
            )
            .await?;
        }
        css_module_lexer::Dependency::LocalKeyframesDecl { name, range, .. } => {
          if !animation_enabled {
            continue;
          }
          self
            .handle_local_ident_declaration(
              name,
              range,
              resource_data,
              compiler_options,
              &mut css_exports,
              &mut css_local_names,
              &mut dependencies,
            )
            .await?;
        }
        css_module_lexer::Dependency::Composes {
          local_classes,
          names,
          from,
          range,
        } => {
          let local_classes = local_classes
            .into_iter()
            .map(|s| unescape(s).to_string())
            .collect::<Vec<_>>();
          let names = names
            .into_iter()
            .map(|s| unescape(s).to_string())
            .collect::<Vec<_>>();

          let mut dep_id = None;
          if let Some(from) = from
            && from != "global"
          {
            let from = from.trim_matches(|c| c == '\'' || c == '"');
            let dep = CssComposeDependency::new(
              from.to_string(),
              names.iter().map(|s| s.to_owned().into()).collect(),
              DependencyRange::new(range.start, range.end),
              import_mode,
            );
            dep_id = Some(*dep.id());
            dependencies.push(Box::new(dep));
          } else if from.is_none() {
            dependencies.push(Box::new(CssSelfReferenceLocalIdentDependency::new(
              names.clone(),
              vec![],
            )));
          }

          let convention = self.convention();
          let exports = css_exports.get_or_insert_default();
          for name in names {
            for local_class in local_classes.iter() {
              let convention_names = export_locals_convention(&name, convention);
              let convention_local_class = export_locals_convention(local_class, convention);

              for (convention_name, local_class) in
                convention_names.into_iter().zip(convention_local_class)
              {
                if let Some(existing) = exports.get(name.as_str())
                  && from.is_none()
                {
                  let existing = existing.clone();
                  exports
                    .get_mut(local_class.as_str())
                    .expect("composes local class must already added to exports")
                    .extend(existing);
                } else {
                  exports
                    .get_mut(local_class.as_str())
                    .expect("composes local class must already added to exports")
                    .insert(CssExport {
                      ident: convention_name.clone(),
                      orig_name: name.clone(),
                      from: from
                        .filter(|f| *f != "global")
                        .map(|f| f.trim_matches(|c| c == '\'' || c == '"').to_string()),
                      id: dep_id,
                    });
                }
              }
            }
          }
        }
        css_module_lexer::Dependency::ICSSExportValue { prop, value } => {
          let exports = css_exports.get_or_insert_default();
          let convention = self.convention();
          let convention_names = export_locals_convention(prop, convention);
          let value = REGEX_IS_COMMENTS.replace_all(value, "");
          let trimmed_value = value.trim();
          for name in convention_names.iter() {
            update_css_exports(
              exports,
              name.to_owned(),
              if let Some(import_ref) = icss_imports.get(trimmed_value) {
                CssExport {
                  ident: import_ref.import_name.clone(),
                  from: Some(import_ref.request.clone()),
                  id: None,
                  orig_name: prop.to_string(),
                }
              } else {
                CssExport {
                  ident: value.to_string(),
                  from: None,
                  id: None,
                  orig_name: prop.to_string(),
                }
              },
            );
          }
          dependencies.push(Box::new(CssExportDependency::new(convention_names)));
        }
        css_module_lexer::Dependency::ICSSImportFrom { path } => {
          icss_import_request = Some(path.trim_matches(|c| c == '\'' || c == '"').to_string());
        }
        css_module_lexer::Dependency::ICSSImportValue { prop, value } => {
          let Some(request) = icss_import_request.clone() else {
            continue;
          };
          let import_name = value.trim().to_string();
          icss_imports.insert(
            prop.to_string(),
            IcssImportReference {
              request: request.clone(),
              import_name: import_name.clone(),
            },
          );
          dependencies.push(Box::new(CssIcssImportDependency::new(
            request,
            import_name,
            prop.to_string(),
            None,
            import_mode,
          )));
        }
        css_module_lexer::Dependency::LocalContainer { name, range, .. } => {
          if !container_enabled {
            continue;
          }
          self
            .handle_local_ident_usage(
              name,
              range,
              resource_data,
              compiler_options,
              &mut css_exports,
              &mut dependencies,
            )
            .await?;
        }
        css_module_lexer::Dependency::LocalContainerDecl { name, range, .. } => {
          if !container_enabled {
            continue;
          }
          self
            .handle_local_ident_declaration(
              name,
              range,
              resource_data,
              compiler_options,
              &mut css_exports,
              &mut css_local_names,
              &mut dependencies,
            )
            .await?;
        }
        css_module_lexer::Dependency::LocalCounterStyle { name, range, .. } => {
          if !custom_idents_enabled {
            continue;
          }
          self
            .handle_local_ident_usage(
              name,
              range,
              resource_data,
              compiler_options,
              &mut css_exports,
              &mut dependencies,
            )
            .await?;
        }
        css_module_lexer::Dependency::LocalCounterStyleDecl { name, range, .. } => {
          if !custom_idents_enabled {
            continue;
          }
          self
            .handle_local_ident_declaration(
              name,
              range,
              resource_data,
              compiler_options,
              &mut css_exports,
              &mut css_local_names,
              &mut dependencies,
            )
            .await?;
        }
        css_module_lexer::Dependency::LocalFontPalette { name, range, .. } => {
          if !custom_idents_enabled {
            continue;
          }
          self
            .handle_local_ident_usage(
              name,
              range,
              resource_data,
              compiler_options,
              &mut css_exports,
              &mut dependencies,
            )
            .await?;
        }
        css_module_lexer::Dependency::LocalFontPaletteDecl { name, range, .. } => {
          if !custom_idents_enabled {
            continue;
          }
          self
            .handle_local_ident_declaration(
              name,
              range,
              resource_data,
              compiler_options,
              &mut css_exports,
              &mut css_local_names,
              &mut dependencies,
            )
            .await?;
        }
        css_module_lexer::Dependency::LocalVar { name, range, .. } => {
          if !dashed_idents_enabled {
            continue;
          }
          self
            .handle_local_ident_usage(
              name,
              range,
              resource_data,
              compiler_options,
              &mut css_exports,
              &mut dependencies,
            )
            .await?;
        }
        css_module_lexer::Dependency::LocalVarDecl { name, range, .. }
        | css_module_lexer::Dependency::LocalPropertyDecl { name, range, .. } => {
          if !dashed_idents_enabled {
            continue;
          }
          self
            .handle_local_ident_declaration(
              name,
              range,
              resource_data,
              compiler_options,
              &mut css_exports,
              &mut css_local_names,
              &mut dependencies,
            )
            .await?;
        }
        css_module_lexer::Dependency::LocalFunction { name, range, .. } => {
          if !function_enabled {
            continue;
          }
          self
            .handle_local_ident_usage(
              name,
              range,
              resource_data,
              compiler_options,
              &mut css_exports,
              &mut dependencies,
            )
            .await?;
        }
        css_module_lexer::Dependency::LocalFunctionDecl { name, range, .. } => {
          if !function_enabled {
            continue;
          }
          self
            .handle_local_ident_declaration(
              name,
              range,
              resource_data,
              compiler_options,
              &mut css_exports,
              &mut css_local_names,
              &mut dependencies,
            )
            .await?;
        }
        css_module_lexer::Dependency::LocalGrid { name, range, .. } => {
          if !grid_enabled {
            continue;
          }
          self
            .handle_local_ident_usage(
              name,
              range,
              resource_data,
              compiler_options,
              &mut css_exports,
              &mut dependencies,
            )
            .await?;
        }
        css_module_lexer::Dependency::LocalGridDecl { name, range, .. } => {
          if !grid_enabled {
            continue;
          }
          self
            .handle_local_ident_declaration(
              name,
              range,
              resource_data,
              compiler_options,
              &mut css_exports,
              &mut css_local_names,
              &mut dependencies,
            )
            .await?;
        }
      }
    }

    for (reference, range) in
      collect_icss_import_usage_ranges(&source_code, &icss_imports, &icss_removed_ranges)
    {
      dependencies.push(Box::new(CssIcssImportDependency::new(
        reference.request,
        reference.import_name,
        String::new(),
        Some(range),
        import_mode,
      )));
    }

    for warning in warnings {
      let range = warning.range();
      let error = css_parsing_traceable_error(
        get_source_code(),
        range.start,
        range.end,
        warning.to_string(),
        if matches!(
          warning.kind(),
          css_module_lexer::WarningKind::NotPrecededAtImport
        ) {
          Severity::Error
        } else {
          Severity::Warning
        },
      );
      diagnostics.push(error.into());
    }

    if matches!(
      self.export_type(),
      Some(CssExportType::Text) | Some(CssExportType::CssStyleSheet)
    ) {
      dependencies.push(Box::new(StaticExportsDependency::new(
        StaticExportsSpec::Array(vec!["default".into()]),
        false,
      )));
    }

    build_info.css_exports = css_exports;
    build_info.css_local_names = css_local_names;

    Ok(
      ParseResult {
        dependencies,
        blocks: vec![],
        presentational_dependencies,
        code_generation_dependencies,
        source,
        side_effects_bailout: None,
      }
      .with_diagnostic(map_box_diagnostics_to_module_parse_diagnostics(
        diagnostics,
        loaders,
      )),
    )
  }

  #[allow(clippy::unwrap_in_result)]
  async fn generate(
    &self,
    source: &BoxSource,
    module: &dyn rspack_core::Module,
    generate_context: &mut GenerateContext,
  ) -> Result<BoxSource> {
    match generate_context.requested_source_type {
      SourceType::Css => {
        if !self.exports_only() {
          generate_context
            .runtime_template
            .runtime_requirements_mut()
            .insert(RuntimeGlobals::HAS_CSS_MODULES);
        }

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
            } else {
              panic!(
                "Can not find dependency template of {:?}",
                dependency.dependency_template()
              );
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
              let media = CssMedia(media.to_string());
              context.data.insert(media);
            }

            if let Some(supports) = css_import_dep.supports() {
              let supports = CssSupports(supports.to_string());
              context.data.insert(supports);
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
            } else {
              panic!(
                "Can not find dependency template of {:?}",
                dependency.dependency_template()
              );
            }
          });
        };

        generate_context.concatenation_scope = context.concatenation_scope.take();

        Ok(source.boxed())
      }
      SourceType::JavaScript => {
        let generator = CssModuleGenerator::new(
          source,
          module,
          generate_context,
          self.hot,
          self.export_type().clone(),
          self.es_module(),
          self.exports_only(),
        );
        let source = generator.generate_javascript_source();
        Ok(source)
      }
      _ => panic!(
        "Unsupported source type: {:?}",
        generate_context.requested_source_type
      ),
    }
  }

  fn get_concatenation_bailout_reason(
    &self,
    _module: &dyn rspack_core::Module,
    _mg: &ModuleGraph,
    _cg: &ChunkGraph,
  ) -> Option<Cow<'static, str>> {
    if self.exports_only() {
      None
    } else {
      // CSS Module cannot be concatenated as it must appear in css chunk, if it's
      // concatenated, it will be removed from module graph
      Some("Module Concatenation is not implemented for CssParserAndGenerator".into())
    }
  }

  async fn get_runtime_hash(
    &self,
    _module: &NormalModule,
    compilation: &Compilation,
    _runtime: Option<&RuntimeSpec>,
  ) -> Result<RspackHashDigest> {
    let mut hasher = RspackHash::from(&compilation.options.output);
    self.es_module().dyn_hash(&mut hasher);
    self.exports_only().dyn_hash(&mut hasher);
    Ok(hasher.digest(&compilation.options.output.hash_digest))
  }
}
