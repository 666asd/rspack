use rayon::prelude::*;
use rspack_core::{
  ChunkGraph, ChunkInitFragments, ChunkKind, ChunkUkey, CodeGenerationPublicPathAutoReplace,
  Compilation, Module, RuntimeCodeTemplate, RuntimeGlobals, RuntimeModuleGenerateContext,
  RuntimeProxyMetadata, RuntimeVariable, SourceType,
  chunk_graph_chunk::ChunkIdSet,
  get_undo_path, property_access,
  rspack_sources::{
    BoxSource, ConcatSource, OriginalSource, RawStringSource, ReplaceSource, Source, SourceExt,
  },
  runtime_mode::RuntimeMode,
};
use rspack_error::{Result, ToStringResultToRspackResultExt};

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
        r.contains(RuntimeGlobals::REQUIRE)
          || r.contains(RuntimeGlobals::REQUIRE_SCOPE)
          || (compilation.options.experiments.runtime_mode == RuntimeMode::Rspack
            && !r.renderable_require_scope().is_empty())
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
        args.push(
          if compilation.options.experiments.runtime_mode == RuntimeMode::Rspack {
            runtime_template.render_runtime_variable(&RuntimeVariable::Context)
          } else {
            runtime_template.render_runtime_globals(&RuntimeGlobals::REQUIRE)
          },
        );
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

fn runtime_context_metadata<'a>(
  compilation: &'a Compilation,
  chunk_ukey: &ChunkUkey,
) -> Option<&'a RuntimeProxyMetadata> {
  if compilation.options.experiments.runtime_mode != RuntimeMode::Rspack {
    return None;
  }

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

pub fn should_render_runtime_context(compilation: &Compilation, chunk_ukey: &ChunkUkey) -> bool {
  if compilation.options.experiments.runtime_mode != RuntimeMode::Rspack {
    return false;
  }

  let chunk = compilation
    .build_chunk_graph_artifact
    .chunk_by_ukey
    .expect_get(chunk_ukey);
  if chunk.kind() == ChunkKind::HotUpdate {
    return false;
  }

  runtime_context_metadata(compilation, chunk_ukey).is_some_and(|metadata| {
    !metadata.lexical_fields().is_empty() || !metadata.context_fields().is_empty()
  }) || should_render_runtime_context_require(compilation, chunk_ukey)
}

pub fn should_render_runtime_context_require(
  compilation: &Compilation,
  chunk_ukey: &ChunkUkey,
) -> bool {
  let runtime_requirements = compilation
    .cgc_runtime_requirements_artifact
    .get(chunk_ukey)
    .copied()
    .unwrap_or_default();
  runtime_requirements.intersects(
    RuntimeGlobals::REQUIRE | RuntimeGlobals::INTERCEPT_MODULE_EXECUTION | RuntimeGlobals::MODULE,
  ) || runtime_requirements.contains(RuntimeGlobals::REQUIRE)
    || {
      let chunk = compilation
        .build_chunk_graph_artifact
        .chunk_by_ukey
        .expect_get(chunk_ukey);
      compilation
        .build_chunk_graph_artifact
        .chunk_graph
        .get_chunk_modules_identifier(chunk_ukey)
        .iter()
        .any(|module_identifier| {
          ChunkGraph::get_module_runtime_requirements(
            compilation,
            *module_identifier,
            chunk.runtime(),
          )
          .is_some_and(|runtime_requirements| {
            runtime_requirements.contains(RuntimeGlobals::REQUIRE)
              || runtime_requirements.contains(RuntimeGlobals::REQUIRE_SCOPE)
          })
        })
    }
}

fn runtime_globals_in_order(runtime_globals: RuntimeGlobals) -> Vec<RuntimeGlobals> {
  RuntimeGlobals::all()
    .iter_names()
    .filter_map(|(_, value)| runtime_globals.contains(value).then_some(value))
    .collect()
}

fn render_runtime_context_declarations(
  compilation: &Compilation,
  chunk_ukey: &ChunkUkey,
  runtime_template: &RuntimeCodeTemplate<'_>,
  render_lexical_fields: bool,
  render_context_object: bool,
) -> Option<BoxSource> {
  if !should_render_runtime_context(compilation, chunk_ukey) {
    return None;
  }

  let mut sources = ConcatSource::default();
  if render_lexical_fields && let Some(metadata) = runtime_context_metadata(compilation, chunk_ukey)
  {
    for runtime_global in runtime_globals_in_order(metadata.lexical_fields()) {
      if let Some(name) = runtime_global.to_lexical_name() {
        sources.add(RawStringSource::from(format!("var {name};\n")));
      }
    }
  }
  if !render_context_object {
    return Some(sources.boxed());
  }

  let runtime_context = runtime_template.render_runtime_variable(&RuntimeVariable::Context);
  sources.add(RawStringSource::from(format!(
    "var {runtime_context} = typeof {runtime_context} !== \"undefined\" ? {runtime_context} : {{}};\n"
  )));
  if should_render_runtime_context_require(compilation, chunk_ukey) {
    sources.add(RawStringSource::from(format!(
      "if (!{runtime_context}.r && typeof {} !== \"undefined\") {runtime_context}.r = {};\n",
      runtime_template.render_runtime_variable(&RuntimeVariable::Require),
      runtime_template.render_runtime_variable(&RuntimeVariable::Require)
    )));
  }

  Some(sources.boxed())
}

fn render_runtime_context_fields(
  compilation: &Compilation,
  chunk_ukey: &ChunkUkey,
  runtime_template: &RuntimeCodeTemplate<'_>,
  render_setter_fields: bool,
  render_plain_fields: bool,
) -> Option<BoxSource> {
  if !runtime_template.uses_lexical_runtime_globals() {
    return None;
  }

  let metadata = runtime_context_metadata(compilation, chunk_ukey)?;
  let mut sources = ConcatSource::default();
  let runtime_context = runtime_template.render_runtime_variable(&RuntimeVariable::Context);

  for runtime_global in runtime_globals_in_order(metadata.context_fields()) {
    let has_setter = metadata.context_setter_fields.contains(runtime_global);
    if (has_setter && !render_setter_fields) || (!has_setter && !render_plain_fields) {
      continue;
    }

    let Some(key) = runtime_global.context_property_name() else {
      continue;
    };
    let Some(lexical_name) = runtime_global.to_lexical_name() else {
      continue;
    };

    if has_setter {
      sources.add(RawStringSource::from(format!(
        "var __rspack_context_field = Object.getOwnPropertyDescriptor({}, {}); if (__rspack_context_field && Object.prototype.hasOwnProperty.call(__rspack_context_field, \"value\")) {} = __rspack_context_field.value;\n",
        runtime_context,
        rspack_util::json_stringify(key),
        lexical_name
      )));
      let getter_value = if runtime_global == RuntimeGlobals::CREATE_FAKE_NAMESPACE_OBJECT {
        format!(
          "function(value, mode) {{ return {lexical_name}.call(typeof this === \"function\" ? this : this.r, value, mode); }}"
        )
      } else if runtime_global == RuntimeGlobals::ENSURE_CHUNK {
        format!("this.r && this.r.e ? this.r.e : {lexical_name}")
      } else {
        lexical_name.to_string()
      };
      sources.add(RawStringSource::from(format!(
        "Object.defineProperty({}, {}, {{ configurable: true, get: function() {{ return {}; }}, set: function(value) {{ {} = value; }} }});\n",
        runtime_context,
        rspack_util::json_stringify(key),
        getter_value,
        lexical_name
      )));
      if let Some(require_key) = runtime_global.property_name() {
        let require_getter_value = if runtime_global == RuntimeGlobals::CREATE_FAKE_NAMESPACE_OBJECT
        {
          format!(
            "function(value, mode) {{ return {lexical_name}.call({runtime_context}.r, value, mode); }}"
          )
        } else {
          lexical_name.to_string()
        };
        sources.add(RawStringSource::from(format!(
          "if ({}.r) Object.defineProperty({}.r, {}, {{ configurable: true, get: function() {{ return {}; }}, set: function(value) {{ {} = value; }} }});\n",
          runtime_context,
          runtime_context,
          rspack_util::json_stringify(require_key),
          require_getter_value,
          lexical_name
        )));
      }
    } else {
      let value = if runtime_global == RuntimeGlobals::CREATE_FAKE_NAMESPACE_OBJECT {
        format!(
          "function(value, mode) {{ return {lexical_name}.call({runtime_context}.r, value, mode); }}"
        )
      } else {
        lexical_name.to_string()
      };
      sources.add(RawStringSource::from(format!(
        "{}{} = {};\n",
        runtime_context,
        property_access([key], 0),
        value
      )));
    }
  }

  Some(sources.boxed())
}

pub async fn render_chunk_runtime_modules(
  compilation: &Compilation,
  chunk_ukey: &ChunkUkey,
  runtime_template: &RuntimeCodeTemplate<'_>,
) -> Result<BoxSource> {
  let runtime_modules_sources =
    render_runtime_modules(compilation, chunk_ukey, runtime_template).await?;
  if runtime_modules_sources.source().is_empty()
    && !should_render_runtime_context(compilation, chunk_ukey)
  {
    return Ok(runtime_modules_sources);
  }

  let mut sources = ConcatSource::default();
  let runtime_module_arguments = if runtime_template.uses_runtime_context() {
    runtime_template.render_runtime_variable(&RuntimeVariable::Context)
  } else {
    runtime_template.render_runtime_globals(&RuntimeGlobals::REQUIRE)
  };
  sources.add(RawStringSource::from(format!(
    "function({runtime_module_arguments}) {{\n",
  )));
  sources.add(runtime_modules_sources);
  sources.add(RawStringSource::from_static("\n}\n"));
  Ok(sources.boxed())
}

pub async fn render_runtime_modules(
  compilation: &Compilation,
  chunk_ukey: &ChunkUkey,
  runtime_template: &RuntimeCodeTemplate<'_>,
) -> Result<BoxSource> {
  let mut sources = ConcatSource::default();
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
        let s = unsafe { token.used((compilation, source, module, runtime_template)) };
        s.spawn(
          |(compilation, source, module, runtime_template)| async move {
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
            if !(module.full_hash()
              || module.dependent_hash()
              || (runtime_template.uses_runtime_context()
                && !runtime_template.uses_lexical_runtime_globals()))
            {
              sources.add(source.clone());
            } else {
              let source = if let Some(custom_source) = module.get_custom_source() {
                RawStringSource::from(custom_source).boxed()
              } else {
                let context = RuntimeModuleGenerateContext {
                  compilation,
                  runtime_template,
                };
                let source_str = module.generate(&context).await?;
                if module.get_source_map_kind().enabled() {
                  OriginalSource::new(source_str, module.identifier().as_str()).boxed()
                } else {
                  RawStringSource::from(source_str).boxed()
                }
              };
              sources.add(source);
            }
            if module.should_isolate() {
              sources.add(RawStringSource::from(if supports_arrow_function {
                "\n})();\n"
              } else {
                "\n}();\n"
              }));
            }
            Ok(sources)
          },
        );
      })
  })
  .await
  .into_iter()
  .map(|r| r.to_rspack_result())
  .collect::<Result<Vec<_>>>()?;

  let isolate_runtime_context = runtime_template.uses_runtime_context()
    && runtime_template.uses_lexical_runtime_globals()
    && !compilation.options.output.module
    && should_render_runtime_context(compilation, chunk_ukey)
    && runtime_context_metadata(compilation, chunk_ukey).is_some_and(|metadata| {
      !metadata.lexical_fields().is_empty() || !metadata.context_fields().is_empty()
    });

  if isolate_runtime_context {
    if let Some(context_declarations) =
      render_runtime_context_declarations(compilation, chunk_ukey, runtime_template, false, true)
    {
      sources.add(context_declarations);
    }
    let runtime_context = runtime_template.render_runtime_variable(&RuntimeVariable::Context);
    sources.add(RawStringSource::from(format!(
      "(function({runtime_context}) {{\n"
    )));
    if let Some(context_declarations) =
      render_runtime_context_declarations(compilation, chunk_ukey, runtime_template, true, false)
    {
      sources.add(context_declarations);
    }
  } else if let Some(context_declarations) =
    render_runtime_context_declarations(compilation, chunk_ukey, runtime_template, true, true)
  {
    sources.add(context_declarations);
  }
  if let Some(context_fields) =
    render_runtime_context_fields(compilation, chunk_ukey, runtime_template, true, false)
  {
    sources.add(context_fields);
  }
  for runtime_module_source in runtime_module_sources {
    sources.add(runtime_module_source?);
  }
  if runtime_template.uses_runtime_context()
    && should_render_runtime_context_require(compilation, chunk_ukey)
  {
    let runtime_context = runtime_template.render_runtime_variable(&RuntimeVariable::Context);
    let require = runtime_template.render_runtime_variable(&RuntimeVariable::Require);
    sources.add(RawStringSource::from(format!(
      "if (typeof {require} !== \"undefined\") {runtime_context}.r = {require};\n"
    )));
    if let Some(context_fields) =
      render_runtime_context_fields(compilation, chunk_ukey, runtime_template, true, false)
    {
      sources.add(context_fields);
    }
  }
  if let Some(context_fields) =
    render_runtime_context_fields(compilation, chunk_ukey, runtime_template, false, true)
  {
    sources.add(context_fields);
  }
  if isolate_runtime_context {
    let runtime_context = runtime_template.render_runtime_variable(&RuntimeVariable::Context);
    sources.add(RawStringSource::from(format!(
      "\n}})({runtime_context});\n"
    )));
  }

  Ok(sources.boxed())
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
