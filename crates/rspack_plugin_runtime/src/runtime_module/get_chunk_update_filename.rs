use rspack_core::{
  AssetHashRecord, Compilation, PathData, RuntimeGlobals, RuntimeModule,
  RuntimeModuleGenerateContext, RuntimeTemplate, SourceType, get_filename_without_hash_length,
  has_hash_placeholder, impl_runtime_module,
};

use super::RuntimeContentHashContext;

// TODO workaround for get_chunk_update_filename
#[impl_runtime_module]
#[derive(Debug)]
pub struct GetChunkUpdateFilenameRuntimeModule {}

impl GetChunkUpdateFilenameRuntimeModule {
  pub fn new(runtime_template: &RuntimeTemplate) -> Self {
    Self::with_default(runtime_template)
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
      let (filename, hash_len_map) =
        get_filename_without_hash_length(&compilation.options.output.hot_update_chunk_filename);
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
          hash_context.mark_content_hash(hash, Some(chunk.ukey()), Some(SourceType::JavaScript))
        });
      let filename = compilation
        .get_path(
          &filename,
          PathData::default()
            .chunk_hash_optional(chunk.rendered_hash(
              &compilation.chunk_hashes_artifact,
              compilation.options.output.hash_digest_length,
            ))
            .chunk_name_optional(chunk.name_for_filename_template())
            .content_hash_optional(marked_content_hash.as_deref())
            .hash(
              format!(
                "' + {}() + '",
                runtime_template.render_runtime_globals(&RuntimeGlobals::GET_FULL_HASH)
              )
              .as_str(),
            )
            .id("' + chunkId + '")
            .runtime(chunk.runtime().as_str()),
        )
        .await?;

      let source = runtime_template.render(
        &self.id,
        Some(serde_json::json!({
          "_filename": format!("'{}'", filename),
        })),
      )?;

      Ok(hash_context.into_recorded_source(source))
    } else {
      unreachable!("should attach chunk for get_main_filename")
    }
  }
}

#[async_trait::async_trait]
impl RuntimeModule for GetChunkUpdateFilenameRuntimeModule {
  fn template(&self) -> Vec<(String, String)> {
    vec![(
      self.id.to_string(),
      include_str!("runtime/get_chunk_update_filename.ejs").to_string(),
    )]
  }

  async fn generate(
    &self,
    context: &RuntimeModuleGenerateContext<'_>,
  ) -> rspack_error::Result<String> {
    self
      .generate_with_real_content_hashes(context)
      .await
      .map(|(source, _)| source)
  }

  async fn generate_real_content_hashes(
    &self,
    context: &RuntimeModuleGenerateContext<'_>,
  ) -> rspack_error::Result<AssetHashRecord> {
    self
      .generate_with_real_content_hashes(context)
      .await
      .map(|(_, record)| record)
  }

  fn additional_runtime_requirements(&self, compilation: &Compilation) -> RuntimeGlobals {
    if has_hash_placeholder(
      compilation
        .options
        .output
        .hot_update_chunk_filename
        .as_str(),
    ) {
      RuntimeGlobals::GET_FULL_HASH
    } else {
      RuntimeGlobals::default()
    }
  }
}
