use rayon::prelude::*;
use rspack_core::{
  ChunkGraph, ChunkInitFragments, ChunkUkey, CodeGenerationPublicPathAutoReplace, Compilation,
  Module, ModuleCodeGenerationContext, RuntimeCodeTemplate, RuntimeGlobalRenderMode,
  RuntimeGlobals, RuntimeProxyMetadata, RuntimeVariable, SourceType,
  chunk_graph_chunk::ChunkIdSet,
  get_undo_path,
  rspack_sources::{BoxSource, ConcatSource, RawStringSource, ReplaceSource, Source, SourceExt},
  runtime_globals_property_name, runtime_globals_to_lexical_variable,
};
use rspack_error::{Result, ToStringResultToRspackResultExt};
use rspack_util::json_stringify_str;

use crate::{JavascriptModulesPluginHooks, RenderSource};

pub const AUTO_PUBLIC_PATH_PLACEHOLDER: &str = "__RSPACK_PLUGIN_ASSET_AUTO_PUBLIC_PATH__";

pub async fn render_chunk_modules(
  compilation: &Compilation,
  chunk_ukey: &ChunkUkey,
  ordered_modules: &Vec<&dyn Module>,
  all_strict: bool,
  output_path: &str,
  hooks: &JavascriptModulesPluginHooks,
  runtime_template: &RuntimeCodeTemplate<'_>,
) -> Result<Option<(BoxSource, ChunkInitFragments)>> {
  let module_sources = rspack_parallel::scope::<_, _>(|token| {
    ordered_modules.iter().for_each(|module| {
      let s = unsafe {
        token.used((
          compilation,
          chunk_ukey,
          module,
          all_strict,
          output_path,
          hooks,
          runtime_template
        ))
      };
      s.spawn(
        |(compilation, chunk_ukey, module, all_strict, output_path, hooks, runtime_template)| async move {
          render_module(
            compilation,
            chunk_ukey,
            *module,
            all_strict,
            true,
            output_path,
            hooks,
            runtime_template
          )
          .await
          .map(|result| result.map(|(s, f, a)| (module.identifier(), s, f, a)))
        },
      );
    });
  })
  .await
  .into_iter()
  .map(|r| r.to_rspack_result())
  .collect::<Result<Vec<_>>>()?;

  let mut module_code_array = vec![];
  for item in module_sources {
    if let Some(i) = item? {
      module_code_array.push(i);
    }
  }

  if module_code_array.is_empty() {
    return Ok(None);
  }

  module_code_array.sort_unstable_by_key(|(module_identifier, _, _, _)| *module_identifier);

  let chunk_init_fragments = module_code_array.iter().fold(
    ChunkInitFragments::default(),
    |mut chunk_init_fragments, (_, _, fragments, additional_fragments)| {
      chunk_init_fragments.extend((*fragments).clone());
      chunk_init_fragments.extend(additional_fragments.clone());
      chunk_init_fragments
    },
  );

  let module_sources: Vec<_> = module_code_array
    .into_iter()
    .map(|(_, source, _, _)| source)
    .collect();
  let module_sources = module_sources
    .into_par_iter()
    .fold(ConcatSource::default, |mut output, source| {
      output.add(source);
      output
    })
    .collect::<Vec<ConcatSource>>();

  let mut sources = ConcatSource::default();
  sources.add(RawStringSource::from_static("{\n"));
  sources.add(ConcatSource::new(module_sources));
  sources.add(RawStringSource::from_static("\n}"));

  Ok(Some((sources.boxed(), chunk_init_fragments)))
}

#[allow(clippy::too_many_arguments)]
pub async fn render_module(
  compilation: &Compilation,
  chunk_ukey: &ChunkUkey,
  module: &dyn Module,
  all_strict: bool,
  factory: bool,
  output_path: &str,
  hooks: &JavascriptModulesPluginHooks,
  runtime_template: &RuntimeCodeTemplate<'_>,
) -> Result<Option<(BoxSource, ChunkInitFragments, ChunkInitFragments)>> {
  let chunk = compilation
    .build_chunk_graph_artifact
    .chunk_by_ukey
    .expect_get(chunk_ukey);
  let code_gen_result = compilation
    .code_generation_results
    .get(&module.identifier(), Some(chunk.runtime()));
  let Some(origin_source) = code_gen_result.get(&SourceType::JavaScript) else {
    return Ok(None);
  };

  let mut module_chunk_init_fragments = match code_gen_result.data.get::<ChunkInitFragments>() {
    Some(fragments) => fragments.clone(),
    None => ChunkInitFragments::default(),
  };

  let mut render_source = if code_gen_result
    .data
    .get::<CodeGenerationPublicPathAutoReplace>()
    .is_some()
  {
    let content = origin_source.source().into_string_lossy();
    let len = AUTO_PUBLIC_PATH_PLACEHOLDER.len();
    let auto_public_path_matches: Vec<_> = content
      .match_indices(AUTO_PUBLIC_PATH_PLACEHOLDER)
      .map(|(index, _)| (index, index + len))
      .collect();
    if !auto_public_path_matches.is_empty() {
      let mut replace = ReplaceSource::new(origin_source.clone());
      for (start, end) in auto_public_path_matches {
        let relative = get_undo_path(
          output_path,
          compilation.options.output.path.to_string(),
          true,
        );
        replace.replace(start as u32, end as u32, relative, None);
      }
      RenderSource {
        source: replace.boxed(),
      }
    } else {
      RenderSource {
        source: origin_source.clone(),
      }
    }
  } else {
    RenderSource {
      source: origin_source.clone(),
    }
  };

  /*
  If supports method shorthand, render function factory as:
  "./module.js"(module) { code }
  Otherwise render as:
  "./module.js": (function(module) { code })
  */
  let use_method_shorthand = compilation
    .options
    .output
    .environment
    .supports_method_shorthand();

  hooks
    .render_module_content
    .call(
      compilation,
      chunk_ukey,
      module,
      &mut render_source,
      &mut module_chunk_init_fragments,
      runtime_template,
    )
    .await?;

  let sources = if factory {
    let mut sources = ConcatSource::default();
    let module_id =
      ChunkGraph::get_module_id(&compilation.module_ids_artifact, module.identifier())
        .expect("should have module_id in render_module");
    sources.add(RawStringSource::from(rspack_util::json_stringify(
      module_id,
    )));

    let mut post_module_container = {
      let runtime_requirements = ChunkGraph::get_module_runtime_requirements(
        compilation,
        module.identifier(),
        chunk.runtime(),
      );

      let need_module = runtime_requirements.is_some_and(|r| r.contains(RuntimeGlobals::MODULE));
      let need_exports = runtime_requirements.is_some_and(|r| r.contains(RuntimeGlobals::EXPORTS));
      let need_require = runtime_requirements.is_some_and(|r| {
        r.contains(RuntimeGlobals::REQUIRE) || r.contains(RuntimeGlobals::REQUIRE_SCOPE)
      });

      let mut args = Vec::new();
      if need_module || need_exports || need_require {
        let module_argument = runtime_template.render_module_argument(module.get_module_argument());
        args.push(if need_module {
          module_argument
        } else {
          format!("__unused_rspack_{module_argument}")
        });
      }

      if need_exports || need_require {
        let exports_argument =
          runtime_template.render_exports_argument(module.get_exports_argument());
        args.push(if need_exports {
          exports_argument
        } else {
          format!("__unused_rspack_{exports_argument}")
        });
      }
      if need_require {
        args.push(runtime_template.render_runtime_globals(&RuntimeGlobals::REQUIRE));
      }

      let mut container_sources = ConcatSource::default();

      if use_method_shorthand {
        container_sources.add(RawStringSource::from(format!("({}) {{\n", args.join(", "))));
      } else {
        container_sources.add(RawStringSource::from(format!(
          ": (function ({}) {{\n",
          args.join(", ")
        )));
      }
      if module.build_info().strict && !all_strict {
        container_sources.add(RawStringSource::from_static("\"use strict\";\n"));
      }
      container_sources.add(render_source.source);

      if use_method_shorthand {
        container_sources.add(RawStringSource::from_static("\n\n},\n"));
      } else {
        container_sources.add(RawStringSource::from_static("\n\n}),\n"));
      }

      RenderSource {
        source: container_sources.boxed(),
      }
    };

    hooks
      .render_module_container
      .call(
        compilation,
        chunk_ukey,
        module,
        &mut post_module_container,
        &mut module_chunk_init_fragments,
        runtime_template,
      )
      .await?;

    let mut post_module_package = post_module_container;

    hooks
      .render_module_package
      .call(
        compilation,
        chunk_ukey,
        module,
        &mut post_module_package,
        &mut module_chunk_init_fragments,
        runtime_template,
      )
      .await?;

    sources.add(post_module_package.source);
    sources.boxed()
  } else {
    hooks
      .render_module_package
      .call(
        compilation,
        chunk_ukey,
        module,
        &mut render_source,
        &mut module_chunk_init_fragments,
        runtime_template,
      )
      .await?;

    render_source.source
  };

  Ok(Some((
    sources,
    code_gen_result.chunk_init_fragments.clone(),
    module_chunk_init_fragments,
  )))
}

pub async fn render_chunk_runtime_modules(
  compilation: &Compilation,
  chunk_ukey: &ChunkUkey,
  runtime_template: &RuntimeCodeTemplate<'_>,
) -> Result<BoxSource> {
  let runtime_modules_sources =
    render_runtime_modules_impl(compilation, chunk_ukey, runtime_template, true, true).await?;
  render_runtime_modules_function(runtime_modules_sources, runtime_template)
}

pub async fn render_chunk_runtime_modules_with_external_runtime_proxy(
  compilation: &Compilation,
  chunk_ukey: &ChunkUkey,
  runtime_template: &RuntimeCodeTemplate<'_>,
) -> Result<BoxSource> {
  let runtime_modules_sources =
    render_runtime_modules_impl(compilation, chunk_ukey, runtime_template, false, false).await?;
  render_runtime_modules_function(runtime_modules_sources, runtime_template)
}

fn render_runtime_modules_function(
  runtime_modules_sources: BoxSource,
  runtime_template: &RuntimeCodeTemplate<'_>,
) -> Result<BoxSource> {
  if runtime_modules_sources.source().is_empty() {
    return Ok(runtime_modules_sources);
  }

  let mut sources = ConcatSource::default();
  sources.add(RawStringSource::from(format!(
    "function({}) {{\n",
    runtime_template.render_runtime_globals(&RuntimeGlobals::REQUIRE),
  )));
  sources.add(runtime_modules_sources);
  sources.add(RawStringSource::from_static("\n}\n"));
  Ok(sources.boxed())
}

pub async fn render_runtime_modules(
  compilation: &Compilation,
  chunk_ukey: &ChunkUkey,
  _runtime_template: &RuntimeCodeTemplate<'_>,
) -> Result<BoxSource> {
  render_runtime_modules_impl(compilation, chunk_ukey, _runtime_template, true, true).await
}

pub async fn render_runtime_modules_with_external_runtime_proxy(
  compilation: &Compilation,
  chunk_ukey: &ChunkUkey,
  runtime_template: &RuntimeCodeTemplate<'_>,
) -> Result<BoxSource> {
  render_runtime_modules_impl(compilation, chunk_ukey, runtime_template, false, false).await
}

async fn render_runtime_modules_impl(
  compilation: &Compilation,
  chunk_ukey: &ChunkUkey,
  _runtime_template: &RuntimeCodeTemplate<'_>,
  include_proxy_declarations: bool,
  declare_runtime_proxy: bool,
) -> Result<BoxSource> {
  let mut sources = ConcatSource::default();
  let runtime_proxy = _runtime_template.render_runtime_variable(&RuntimeVariable::Runtime);
  let use_private_runtime_proxy_scope = include_proxy_declarations
    && declare_runtime_proxy
    && has_runtime_proxy_support(compilation, chunk_ukey);
  let proxy_variable = if use_private_runtime_proxy_scope {
    "__proxy"
  } else {
    &runtime_proxy
  };

  if use_private_runtime_proxy_scope {
    sources.add(RawStringSource::from(format!(
      "{runtime_proxy} = {} {{\n",
      runtime_proxy_iife_expression_start(compilation)
    )));
    if let Some(declarations) = render_runtime_proxy_private_declarations(compilation, chunk_ukey) {
      sources.add(declarations);
    }
    if let Some(initializers) =
      render_runtime_proxy_bootstrap_initializers(compilation, chunk_ukey, _runtime_template)
    {
      sources.add(initializers);
    }
  } else if include_proxy_declarations
    && let Some(declarations) =
      render_runtime_proxy_declarations(compilation, chunk_ukey, _runtime_template)
  {
    sources.add(declarations);
  }
  let has_custom_runtime_module = get_runtime_proxy_metadata(compilation, chunk_ukey)
    .is_some_and(|metadata| metadata.has_custom_runtime_module);
  if has_custom_runtime_module
    && let Some(bridge) = render_runtime_proxy_bridge(
      compilation,
      chunk_ukey,
      _runtime_template,
      declare_runtime_proxy,
      proxy_variable,
    )
  {
    sources.add(bridge);
  }
  let runtime_module_sources = rspack_parallel::scope::<_, Result<_>>(|token| {
    compilation
      .build_chunk_graph_artifact
      .chunk_graph
      .get_chunk_runtime_modules_in_order(chunk_ukey, compilation)
      .map(|(identifier, runtime_module)| {
        (
          compilation
            .runtime_modules_code_generation_source
            .get(identifier)
            .expect("should have runtime module result"),
          runtime_module,
        )
      })
      .for_each(|(source, module)| {
        let s = unsafe { token.used((compilation, source, module)) };
        s.spawn(|(compilation, source, module)| async move {
          let mut sources = ConcatSource::default();
          if source.size() == 0 {
            return Ok(sources);
          }
          sources.add(RawStringSource::from(format!(
            "// {}\n",
            module.identifier()
          )));
          let supports_arrow_function = compilation
            .options
            .output
            .environment
            .supports_arrow_function();
          if module.should_isolate() {
            sources.add(RawStringSource::from(if supports_arrow_function {
              "(() => {\n"
            } else {
              "!function() {\n"
            }));
          }
          if !(module.full_hash() || module.dependent_hash()) {
            sources.add(source.clone());
          } else {
            let render_mode = if compilation
              .options
              .experiments
              .runtime_mode
              .is_runtime_requirements_proxy_enabled()
            {
              RuntimeGlobalRenderMode::LexicalRuntime
            } else {
              RuntimeGlobalRenderMode::RequireProperty
            };
            let mut runtime_template = compilation
              .runtime_template
              .create_module_code_template(render_mode);
            let mut code_generation_context = ModuleCodeGenerationContext {
              compilation,
              runtime: None,
              concatenation_scope: None,
              runtime_template: &mut runtime_template,
            };

            let result = module.code_generation(&mut code_generation_context).await?;
            #[allow(clippy::unwrap_used)]
            let source = result.get(&SourceType::Runtime).unwrap();
            sources.add(source.clone());
          }
          if module.should_isolate() {
            sources.add(RawStringSource::from(if supports_arrow_function {
              "\n})();\n"
            } else {
              "\n}();\n"
            }));
          }
          Ok(sources)
        });
      })
  })
  .await
  .into_iter()
  .map(|r| r.to_rspack_result())
  .collect::<Result<Vec<_>>>()?;

  for runtime_module_source in runtime_module_sources {
    sources.add(runtime_module_source?);
  }

  if let Some(assignments) =
    render_runtime_proxy_proxy_assignments(compilation, chunk_ukey, proxy_variable)
  {
    sources.add(assignments);
  }
  if !has_custom_runtime_module
    && let Some(bridge) = render_runtime_proxy_bridge(
      compilation,
      chunk_ukey,
      _runtime_template,
      declare_runtime_proxy,
      proxy_variable,
    )
  {
    sources.add(bridge);
  }

  if use_private_runtime_proxy_scope {
    sources.add(RawStringSource::from(format!(
      "return __proxy;\n}}{};\n",
      runtime_proxy_iife_expression_end(compilation)
    )));
  }

  Ok(sources.boxed())
}

pub fn has_runtime_proxy_support(compilation: &Compilation, chunk_ukey: &ChunkUkey) -> bool {
  if !compilation
    .options
    .experiments
    .runtime_mode
    .is_runtime_requirements_proxy_enabled()
  {
    return false;
  }

  get_runtime_proxy_metadata(compilation, chunk_ukey).is_some_and(|metadata| {
    !metadata.lexical_fields().is_empty()
      || !metadata.proxy_fields().is_empty()
      || !metadata.write_bridge_fields.is_empty()
  })
}

fn render_runtime_proxy_private_declarations(
  compilation: &Compilation,
  chunk_ukey: &ChunkUkey,
) -> Option<BoxSource> {
  let metadata = get_runtime_proxy_metadata(compilation, chunk_ukey)?;
  if metadata.lexical_fields().is_empty()
    && metadata.proxy_fields().is_empty()
    && metadata.write_bridge_fields.is_empty()
  {
    return None;
  }

  let source = render_runtime_proxy_variable_declarations(
    compilation,
    metadata.lexical_fields(),
    ["__proxy = {}".to_string()],
  )?;
  Some(RawStringSource::from(source).boxed())
}

pub fn render_runtime_proxy_declarations(
  compilation: &Compilation,
  chunk_ukey: &ChunkUkey,
  runtime_template: &RuntimeCodeTemplate<'_>,
) -> Option<BoxSource> {
  if !compilation
    .options
    .experiments
    .runtime_mode
    .is_runtime_requirements_proxy_enabled()
  {
    return None;
  }

  let metadata = get_runtime_proxy_metadata(compilation, chunk_ukey)?;
  if metadata.lexical_fields().is_empty()
    && metadata.proxy_fields().is_empty()
    && metadata.write_bridge_fields.is_empty()
  {
    return None;
  }

  let runtime_proxy = runtime_template.render_runtime_variable(&RuntimeVariable::Runtime);
  let extra_declarators = (!metadata.proxy_fields().is_empty()
    || !metadata.write_bridge_fields.is_empty())
  .then(|| format!("{runtime_proxy} = {{}}"));
  let source = render_runtime_proxy_variable_declarations(
    compilation,
    metadata.lexical_fields(),
    extra_declarators,
  )?;
  Some(RawStringSource::from(source).boxed())
}

pub fn render_runtime_proxy_outer_declarations(
  compilation: &Compilation,
  chunk_ukey: &ChunkUkey,
  runtime_template: &RuntimeCodeTemplate<'_>,
) -> Option<BoxSource> {
  if !compilation
    .options
    .experiments
    .runtime_mode
    .is_runtime_requirements_proxy_enabled()
  {
    return None;
  }

  let metadata = get_runtime_proxy_metadata(compilation, chunk_ukey)?;
  if metadata.proxy_fields().is_empty() && metadata.write_bridge_fields.is_empty() {
    return None;
  }

  let runtime_proxy = runtime_template.render_runtime_variable(&RuntimeVariable::Runtime);
  let source = render_runtime_proxy_variable_declarations(
    compilation,
    RuntimeGlobals::default(),
    [runtime_proxy],
  )?;
  Some(RawStringSource::from(source).boxed())
}

pub fn render_runtime_proxy_bootstrap_initializers(
  compilation: &Compilation,
  chunk_ukey: &ChunkUkey,
  runtime_template: &RuntimeCodeTemplate<'_>,
) -> Option<BoxSource> {
  if !compilation
    .options
    .experiments
    .runtime_mode
    .is_runtime_requirements_proxy_enabled()
  {
    return None;
  }

  let runtime_requirements = ChunkGraph::get_chunk_runtime_requirements(compilation, chunk_ukey);
  let mut source = String::new();
  if runtime_requirements.contains(RuntimeGlobals::MODULE_FACTORIES)
    || runtime_requirements.contains(RuntimeGlobals::MODULE_FACTORIES_ADD_ONLY)
  {
    let lexical =
      runtime_globals_to_lexical_variable(&RuntimeGlobals::MODULE_FACTORIES, &compilation.options);
    let modules = runtime_template.render_runtime_variable(&RuntimeVariable::Modules);
    source.push_str(&format!("{lexical} = {modules};\n"));
  }
  if runtime_requirements.contains(RuntimeGlobals::MODULE_CACHE) {
    let lexical =
      runtime_globals_to_lexical_variable(&RuntimeGlobals::MODULE_CACHE, &compilation.options);
    let module_cache = runtime_template.render_runtime_variable(&RuntimeVariable::ModuleCache);
    source.push_str(&format!("{lexical} = {module_cache};\n"));
  }
  if runtime_requirements.contains(RuntimeGlobals::INTERCEPT_MODULE_EXECUTION) {
    let lexical = runtime_globals_to_lexical_variable(
      &RuntimeGlobals::INTERCEPT_MODULE_EXECUTION,
      &compilation.options,
    );
    source.push_str(&format!("{lexical} = [];\n"));
  }
  (!source.is_empty()).then(|| RawStringSource::from(source).boxed())
}

fn runtime_proxy_bridge_declaration(compilation: &Compilation) -> &'static str {
  if compilation.options.output.environment.supports_const() {
    "let"
  } else {
    "var"
  }
}

fn render_runtime_proxy_variable_declarations(
  compilation: &Compilation,
  lexical_fields: RuntimeGlobals,
  extra_declarators: impl IntoIterator<Item = String>,
) -> Option<String> {
  let mut declarators = lexical_fields
    .iter()
    .map(|runtime_global| {
      runtime_globals_to_lexical_variable(&runtime_global, &compilation.options)
    })
    .chain(extra_declarators)
    .collect::<Vec<_>>();
  if declarators.is_empty() {
    return None;
  }

  declarators.sort_unstable();
  let declaration = runtime_proxy_bridge_declaration(compilation);
  Some(format!("{declaration} {};\n", declarators.join(", ")))
}

fn runtime_proxy_iife_start(compilation: &Compilation) -> &'static str {
  if compilation
    .options
    .output
    .environment
    .supports_arrow_function()
  {
    "(() =>"
  } else {
    "!function()"
  }
}

fn runtime_proxy_iife_end(compilation: &Compilation) -> &'static str {
  if compilation
    .options
    .output
    .environment
    .supports_arrow_function()
  {
    ")()"
  } else {
    "()"
  }
}

fn runtime_proxy_iife_expression_start(compilation: &Compilation) -> &'static str {
  if compilation
    .options
    .output
    .environment
    .supports_arrow_function()
  {
    "(() =>"
  } else {
    "(function()"
  }
}

fn runtime_proxy_iife_expression_end(_compilation: &Compilation) -> &'static str {
  ")()"
}

fn runtime_proxy_define_function(
  compilation: &Compilation,
  runtime_proxy: &str,
  require: &str,
) -> String {
  if compilation
    .options
    .output
    .environment
    .supports_arrow_function()
  {
    format!(
      "(item) => {{ Object.defineProperty({runtime_proxy}, item[0], {{ configurable: true, enumerable: true, get: item[1], set: item[2] }}); Object.defineProperty({require}, item[0], {{ configurable: true, enumerable: true, get: () => {runtime_proxy}[item[0]], set: (value) => {{ {runtime_proxy}[item[0]] = value; }} }}); }}"
    )
  } else {
    format!(
      "function(item) {{ Object.defineProperty({runtime_proxy}, item[0], {{ configurable: true, enumerable: true, get: item[1], set: item[2] }}); Object.defineProperty({require}, item[0], {{ configurable: true, enumerable: true, get: function() {{ return {runtime_proxy}[item[0]]; }}, set: function(value) {{ {runtime_proxy}[item[0]] = value; }} }}); }}"
    )
  }
}

pub fn render_runtime_proxy_bridge(
  compilation: &Compilation,
  chunk_ukey: &ChunkUkey,
  runtime_template: &RuntimeCodeTemplate<'_>,
  _declare_runtime_proxy: bool,
  runtime_proxy: &str,
) -> Option<BoxSource> {
  if !compilation
    .options
    .experiments
    .runtime_mode
    .is_runtime_requirements_proxy_enabled()
  {
    return None;
  }

  let metadata = get_runtime_proxy_metadata(compilation, chunk_ukey)?;
  if metadata.write_bridge_fields.is_empty() {
    return None;
  }

  let require = runtime_template.render_runtime_variable(&RuntimeVariable::Require);
  let entries = metadata
    .write_bridge_fields
    .iter()
    .map(|runtime_global| {
      let key = runtime_globals_property_name(&runtime_global)
        .expect("runtime global should be renderable as proxy property");
      let key = json_stringify_str(key);
      let lexical = runtime_globals_to_lexical_variable(&runtime_global, &compilation.options);
      if compilation
        .options
        .output
        .environment
        .supports_arrow_function()
      {
        format!("[{key}, () => {lexical}, (value) => {{ {lexical} = value; }}]")
      } else {
        format!(
          "[{key}, function() {{ return {lexical}; }}, function(value) {{ {lexical} = value; }}]"
        )
      }
    })
    .collect::<Vec<_>>();
  let declaration = runtime_proxy_bridge_declaration(compilation);
  let define_function = runtime_proxy_define_function(compilation, runtime_proxy, &require);
  let source = format!(
    "{} {{ {declaration} __bridge = [{}]; {declaration} __define = {define_function}; for ({declaration} i = 0; i < __bridge.length; i++) __define(__bridge[i]); }}{};\n",
    runtime_proxy_iife_start(compilation),
    entries.join(","),
    runtime_proxy_iife_end(compilation)
  );
  Some(RawStringSource::from(source).boxed())
}

fn render_runtime_proxy_proxy_assignments(
  compilation: &Compilation,
  chunk_ukey: &ChunkUkey,
  runtime_proxy: &str,
) -> Option<BoxSource> {
  if !compilation
    .options
    .experiments
    .runtime_mode
    .is_runtime_requirements_proxy_enabled()
  {
    return None;
  }

  let metadata = get_runtime_proxy_metadata(compilation, chunk_ukey)?;
  let proxy_assignment_fields = metadata
    .proxy_fields()
    .difference(metadata.write_bridge_fields);
  if proxy_assignment_fields.is_empty() {
    return None;
  }

  let entries = proxy_assignment_fields
    .iter()
    .map(|runtime_global| {
      let key = runtime_globals_property_name(&runtime_global)
        .expect("runtime global should be renderable as proxy property");
      let key = json_stringify_str(key);
      let lexical = runtime_globals_to_lexical_variable(&runtime_global, &compilation.options);
      format!("[{key}, {lexical}]")
    })
    .collect::<Vec<_>>();
  let declaration = runtime_proxy_bridge_declaration(compilation);
  let source = format!(
    "{} {{ {declaration} __fields = [{}]; for ({declaration} i = 0; i < __fields.length; i++) {{ {declaration} item = __fields[i]; if (typeof item[1] !== \"undefined\") {runtime_proxy}[item[0]] = item[1]; }} }}{};\n",
    runtime_proxy_iife_start(compilation),
    entries.join(","),
    runtime_proxy_iife_end(compilation)
  );
  Some(RawStringSource::from(source).boxed())
}

pub fn render_runtime_proxy_external_module_proxy(
  compilation: &Compilation,
  chunk_ukey: &ChunkUkey,
  runtime_template: &RuntimeCodeTemplate<'_>,
  export_expression: &str,
) -> Option<BoxSource> {
  if !compilation
    .options
    .experiments
    .runtime_mode
    .is_runtime_requirements_proxy_enabled()
  {
    return None;
  }

  let metadata = get_runtime_proxy_metadata(compilation, chunk_ukey)?;
  if metadata.proxy_fields().is_empty() && metadata.write_bridge_fields.is_empty() {
    return None;
  }

  let runtime_proxy = runtime_template.render_runtime_variable(&RuntimeVariable::Runtime);
  let declaration = runtime_proxy_bridge_declaration(compilation);
  let set_proxy = if compilation
    .options
    .output
    .environment
    .supports_arrow_function()
  {
    format!("(proxy) => {{ {runtime_proxy} = proxy; }}")
  } else {
    format!("function(proxy) {{ {runtime_proxy} = proxy; }}")
  };
  Some(
    RawStringSource::from(format!(
      "{declaration} {runtime_proxy};\n{export_expression} = {set_proxy};\n"
    ))
    .boxed(),
  )
}

pub fn render_runtime_proxy_external_module_proxy_export(
  compilation: &Compilation,
  chunk_ukey: &ChunkUkey,
  runtime_template: &RuntimeCodeTemplate<'_>,
  export_name: &str,
) -> Option<BoxSource> {
  if !compilation
    .options
    .experiments
    .runtime_mode
    .is_runtime_requirements_proxy_enabled()
  {
    return None;
  }

  let metadata = get_runtime_proxy_metadata(compilation, chunk_ukey)?;
  if metadata.proxy_fields().is_empty() && metadata.write_bridge_fields.is_empty() {
    return None;
  }

  let runtime_proxy = runtime_template.render_runtime_variable(&RuntimeVariable::Runtime);
  let declaration = runtime_proxy_bridge_declaration(compilation);
  let set_proxy = if compilation
    .options
    .output
    .environment
    .supports_arrow_function()
  {
    format!("(proxy) => {{ {runtime_proxy} = proxy; }}")
  } else {
    format!("function(proxy) {{ {runtime_proxy} = proxy; }}")
  };
  Some(
    RawStringSource::from(format!(
      "{declaration} {runtime_proxy};\nexport const {export_name} = {set_proxy};\n"
    ))
    .boxed(),
  )
}

fn get_runtime_proxy_metadata<'a>(
  compilation: &'a Compilation,
  chunk_ukey: &ChunkUkey,
) -> Option<&'a RuntimeProxyMetadata> {
  if let Some(metadata) = compilation.runtime_proxy_metadata_artifact.get(chunk_ukey) {
    return Some(metadata);
  }

  let chunk = compilation
    .build_chunk_graph_artifact
    .chunk_by_ukey
    .expect_get(chunk_ukey);
  compilation
    .runtime_proxy_metadata_artifact
    .iter()
    .find_map(|(runtime_chunk_ukey, metadata)| {
      let runtime_chunk = compilation
        .build_chunk_graph_artifact
        .chunk_by_ukey
        .expect_get(runtime_chunk_ukey);
      runtime_chunk
        .runtime()
        .iter()
        .any(|runtime| chunk.runtime().contains(runtime))
        .then_some(metadata)
    })
}

pub fn stringify_chunks_to_array(chunks: &ChunkIdSet) -> String {
  let mut v = chunks.iter().collect::<Vec<_>>();
  v.sort_unstable();
  rspack_util::json_stringify(&v)
}

pub fn stringify_array(vec: &[String]) -> String {
  format!(
    r#"[{}]"#,
    vec
      .iter()
      .map(|item| format!("\"{item}\""))
      .collect::<Vec<_>>()
      .join(", ")
  )
}

#[cfg(test)]
mod tests {
  use rspack_core::chunk_graph_chunk::ChunkIdSet;

  use super::stringify_chunks_to_array;

  #[test]
  fn stringify_chunks_to_array_uses_chunk_id_serialize() {
    let chunks = ChunkIdSet::from_iter([
      rspack_core::chunk_graph_chunk::ChunkId::from("681"),
      rspack_core::chunk_graph_chunk::ChunkId::from("main"),
    ]);

    assert_eq!(stringify_chunks_to_array(&chunks), "[681,\"main\"]");
  }
}
