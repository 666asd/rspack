use std::sync::LazyLock;

use rspack_core::{
  AssetHashRecord, Compilation, OutputOptions, PathData, RuntimeCodeTemplate, RuntimeGlobals,
  RuntimeModule, RuntimeModuleGenerateContext, RuntimeModuleStage, RuntimeTemplate, SourceType,
  get_filename_without_hash_length, get_js_chunk_filename_template, get_undo_path,
  impl_runtime_module,
};

use crate::{extract_runtime_globals_from_ejs, runtime_module::RuntimeContentHashContext};

static AUTO_PUBLIC_PATH_TEMPLATE: &str = include_str!("runtime/auto_public_path.ejs");
static AUTO_PUBLIC_PATH_RUNTIME_REQUIREMENTS: LazyLock<RuntimeGlobals> =
  LazyLock::new(|| extract_runtime_globals_from_ejs(AUTO_PUBLIC_PATH_TEMPLATE));

#[impl_runtime_module]
#[derive(Debug)]
pub struct AutoPublicPathRuntimeModule {}

impl AutoPublicPathRuntimeModule {
  pub fn new(runtime_template: &RuntimeTemplate) -> Self {
    Self::with_default(runtime_template)
  }

  async fn generate_with_real_content_hashes(
    &self,
    context: &RuntimeModuleGenerateContext<'_>,
  ) -> rspack_error::Result<(String, AssetHashRecord)> {
    let compilation = context.compilation;
    let runtime_template = context.runtime_template;
    let chunk = self.chunk.expect("The chunk should be attached");
    let chunk = compilation
      .build_chunk_graph_artifact
      .chunk_by_ukey
      .expect_get(&chunk);
    let filename_template = get_js_chunk_filename_template(
      chunk,
      &compilation.options.output,
      &compilation.build_chunk_graph_artifact.chunk_group_by_ukey,
    );
    let (filename_template, hash_len_map) = get_filename_without_hash_length(&filename_template);
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
        &filename_template,
        PathData::default()
          .chunk_id_optional(chunk.id().map(|id| id.as_str()))
          .chunk_hash_optional(chunk.rendered_hash(
            &compilation.chunk_hashes_artifact,
            compilation.options.output.hash_digest_length,
          ))
          .chunk_name_optional(chunk.name_for_filename_template())
          .content_hash_optional(marked_content_hash.as_deref()),
      )
      .await?;
    let source = auto_public_path_template(
      runtime_template,
      &self.id,
      &filename,
      &compilation.options.output,
    )?;
    Ok(hash_context.into_recorded_source(source))
  }
}

#[async_trait::async_trait]
impl RuntimeModule for AutoPublicPathRuntimeModule {
  fn stage(&self) -> RuntimeModuleStage {
    RuntimeModuleStage::Attach
  }

  fn template(&self) -> Vec<(String, String)> {
    vec![(self.id.to_string(), AUTO_PUBLIC_PATH_TEMPLATE.to_string())]
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

  fn additional_runtime_requirements(&self, _compilation: &Compilation) -> RuntimeGlobals {
    *AUTO_PUBLIC_PATH_RUNTIME_REQUIREMENTS
  }
}

fn auto_public_path_template(
  runtime_template: &RuntimeCodeTemplate,
  id: &str,
  filename: &str,
  output: &OutputOptions,
) -> rspack_error::Result<String> {
  let output_path = output.path.as_str().to_string();
  let undo_path = get_undo_path(filename, output_path, false);
  let import_meta_name = output.import_meta_name.clone();

  runtime_template.render(
    id,
    Some(serde_json::json!({
      "_script_type": output.script_type,
      "_import_meta_name": import_meta_name,
      "_undo_path": undo_path
    })),
  )
}
