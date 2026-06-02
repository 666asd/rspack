use rspack_core::{
  ChunkGraph, ChunkInitFragments, ChunkUkey, CodeGenerationPublicPathAutoReplace, Compilation,
  Module, ModuleCodeGenerationContext, RuntimeCodeTemplate, RuntimeGlobals, SourceType,
  chunk_graph_chunk::ChunkIdSet,
  get_undo_path,
  rspack_sources::{BoxSource, ConcatSource, RawStringSource, ReplaceSource, Source, SourceExt},
};
use rspack_error::{Result, ToStringResultToRspackResultExt};

use crate::JavascriptModulesPluginHooks;

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

  let mut module_code_array = Vec::with_capacity(module_sources.len());
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

  let mut chunk_modules_source = ConcatSource::with_capacity(module_code_array.len() + 2);
  chunk_modules_source.add(RawStringSource::from_static("{\n"));
  for (_, source, _, _) in module_code_array {
    chunk_modules_source.add(source);
  }
  chunk_modules_source.add(RawStringSource::from_static("\n}"));

  Ok(Some((chunk_modules_source.boxed(), chunk_init_fragments)))
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
      replace.boxed()
    } else {
      origin_source.clone()
    }
  } else {
    origin_source.clone()
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
    let mut sources = ConcatSource::with_capacity(2);
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

      let use_strict = module.build_info().strict && !all_strict;
      let mut container_sources = ConcatSource::with_capacity(if use_strict { 4 } else { 3 });

      if use_method_shorthand {
        container_sources.add(RawStringSource::from(format!("({}) {{\n", args.join(", "))));
      } else {
        container_sources.add(RawStringSource::from(format!(
          ": (function ({}) {{\n",
          args.join(", ")
        )));
      }
      if use_strict {
        container_sources.add(RawStringSource::from_static("\"use strict\";\n"));
      }
      container_sources.add(render_source);

      if use_method_shorthand {
        container_sources.add(RawStringSource::from_static("\n\n},\n"));
      } else {
        container_sources.add(RawStringSource::from_static("\n\n}),\n"));
      }

      container_sources.boxed()
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

    sources.add(post_module_package);
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

    render_source
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
  let runtime_modules_source =
    render_runtime_modules(compilation, chunk_ukey, runtime_template).await?;
  if runtime_modules_source.size() == 0 {
    return Ok(runtime_modules_source);
  }

  let concat_source = ConcatSource::new([
    RawStringSource::from(format!(
      "function({}) {{\n",
      runtime_template.render_runtime_globals(&RuntimeGlobals::REQUIRE),
    ))
    .boxed(),
    runtime_modules_source,
    RawStringSource::from_static("\n}\n").boxed(),
  ]);
  Ok(concat_source.boxed())
}

pub async fn render_runtime_modules(
  compilation: &Compilation,
  chunk_ukey: &ChunkUkey,
  _runtime_template: &RuntimeCodeTemplate<'_>,
) -> Result<BoxSource> {
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
          if source.size() == 0 {
            return Ok(ConcatSource::default());
          }
          let should_isolate = module.should_isolate();
          let capacity = if should_isolate { 4 } else { 2 };
          let mut sources = ConcatSource::with_capacity(capacity);
          sources.add(RawStringSource::from(format!(
            "// {}\n",
            module.identifier()
          )));
          let supports_arrow_function = compilation
            .options
            .output
            .environment
            .supports_arrow_function();
          if should_isolate {
            sources.add(RawStringSource::from(if supports_arrow_function {
              "(() => {\n"
            } else {
              "!function() {\n"
            }));
          }
          if !(module.full_hash() || module.dependent_hash()) {
            sources.add(source.clone());
          } else {
            let mut runtime_template = compilation.runtime_template.create_module_code_template();
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
          if should_isolate {
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

  let mut concat_source = ConcatSource::with_capacity(runtime_module_sources.len());
  for runtime_module_source in runtime_module_sources {
    concat_source.add(runtime_module_source?);
  }

  Ok(concat_source.boxed())
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
