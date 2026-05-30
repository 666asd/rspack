use rspack_core::{
  AssetHashRecord, Compilation, CustomSourceRuntimeModule, Filename, PathData, RuntimeGlobals,
  RuntimeModule, RuntimeModuleGenerateContext, RuntimeTemplate, SourceType,
  get_filename_without_hash_length, has_hash_placeholder, impl_runtime_module,
};

use super::RuntimeContentHashContext;

#[impl_runtime_module]
#[derive(Debug)]
pub struct GetMainFilenameRuntimeModule {
  global: RuntimeGlobals,
  filename: Filename,
}

impl GetMainFilenameRuntimeModule {
  pub fn new(
    runtime_template: &RuntimeTemplate,
    content_type: &'static str,
    global: RuntimeGlobals,
    filename: Filename,
  ) -> Self {
    Self::with_name(
      runtime_template,
      &format!("get_main_filename/{content_type}"),
      global,
      filename,
    )
  }

  async fn generate_with_real_content_hashes(
    &self,
    context: &RuntimeModuleGenerateContext<'_>,
  ) -> rspack_error::Result<(String, AssetHashRecord)> {
    let compilation = context.compilation;
    let runtime_template = context.runtime_template;
    if let Some(chunk_ukey) = self.chunk {
      let chunk = compilation
        .build_chunk_graph_artifact
        .chunk_by_ukey
        .expect_get(&chunk_ukey);
      let (filename, hash_len_map) = get_filename_without_hash_length(&self.filename);
      let mut hash_context = RuntimeContentHashContext::default();
      let marked_content_hash = chunk
        .rendered_content_hash_by_source_type(
          &compilation.chunk_hashes_artifact,
          &SourceType::JavaScript,
          compilation.options.output.hash_digest_length,
        )
        .map(|hash| {
          let hash = match hash_len_map.get("[contenthash]") {
            Some(hash_len) => &hash[..*hash_len],
            None => hash,
          };
          hash_context.mark_content_hash_replacement(
            hash,
            Some(chunk.ukey()),
            Some(SourceType::JavaScript),
          )
        });
      let filename = compilation
        .get_path(
          &filename,
          PathData::default()
            .chunk_id_optional(chunk.id().map(|id| id.as_str()))
            .chunk_hash_optional(chunk.rendered_hash(
              &compilation.chunk_hashes_artifact,
              compilation.options.output.hash_digest_length,
            ))
            .chunk_name_optional(chunk.name_for_filename_template())
            .content_hash_optional(marked_content_hash.as_deref())
            .hash(
              format!(
                "\" + {}() + \"",
                runtime_template.render_runtime_globals(&RuntimeGlobals::GET_FULL_HASH)
              )
              .as_str(),
            )
            .runtime(chunk.runtime().as_str()),
        )
        .await?;

      let source = format!(
        "{} = function () {{
            return \"{}\";
         }};
        ",
        runtime_template.render_runtime_globals(&self.global),
        filename,
      );
      Ok(hash_context.into_recorded_source(source))
    } else {
      unreachable!("should attach chunk for get_main_filename")
    }
  }
}

#[async_trait::async_trait]
impl RuntimeModule for GetMainFilenameRuntimeModule {
  async fn generate(
    &self,
    context: &RuntimeModuleGenerateContext<'_>,
  ) -> rspack_error::Result<String> {
    let compilation = context.compilation;
    let runtime_template = context.runtime_template;
    if let Some(chunk_ukey) = self.chunk {
      let chunk = compilation
        .build_chunk_graph_artifact
        .chunk_by_ukey
        .expect_get(&chunk_ukey);
      let filename = compilation
        .get_path(
          &self.filename,
          PathData::default()
            .chunk_id_optional(chunk.id().map(|id| id.as_str()))
            .chunk_hash_optional(chunk.rendered_hash(
              &compilation.chunk_hashes_artifact,
              compilation.options.output.hash_digest_length,
            ))
            .chunk_name_optional(chunk.name_for_filename_template())
            .content_hash_optional(chunk.rendered_content_hash_by_source_type(
              &compilation.chunk_hashes_artifact,
              &SourceType::JavaScript,
              compilation.options.output.hash_digest_length,
            ))
            .hash(
              format!(
                "\" + {}() + \"",
                runtime_template.render_runtime_globals(&RuntimeGlobals::GET_FULL_HASH)
              )
              .as_str(),
            )
            .runtime(chunk.runtime().as_str()),
        )
        .await?;

      Ok(format!(
        "{} = function () {{
            return \"{}\";
         }};
        ",
        runtime_template.render_runtime_globals(&self.global),
        filename,
      ))
    } else {
      unreachable!("should attach chunk for get_main_filename")
    }
  }

  async fn generate_with_real_content_hashes(
    &self,
    context: &RuntimeModuleGenerateContext<'_>,
  ) -> rspack_error::Result<(String, AssetHashRecord)> {
    if let Some(custom_source) = self.get_custom_source() {
      return Ok((custom_source, AssetHashRecord::default()));
    }
    Self::generate_with_real_content_hashes(self, context).await
  }

  fn additional_runtime_requirements(&self, compilation: &Compilation) -> RuntimeGlobals {
    if has_hash_placeholder(compilation.options.output.hot_update_main_filename.as_str()) {
      RuntimeGlobals::GET_FULL_HASH
    } else {
      RuntimeGlobals::default()
    }
  }
}
