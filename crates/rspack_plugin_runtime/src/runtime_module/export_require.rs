use rspack_core::{
  RuntimeModule, RuntimeModuleGenerateContext, RuntimeTemplate, RuntimeVariable,
  impl_runtime_module,
};

pub static EXPORT_REQUIRE_RUNTIME_MODULE_ID: &str = "export_webpack_require";

#[impl_runtime_module]
#[derive(Debug)]
pub struct ExportRequireRuntimeModule {}

impl ExportRequireRuntimeModule {
  pub fn new(runtime_template: &RuntimeTemplate) -> Self {
    Self::with_name(runtime_template, EXPORT_REQUIRE_RUNTIME_MODULE_ID)
  }
}

#[async_trait::async_trait]
impl RuntimeModule for ExportRequireRuntimeModule {
  async fn generate(
    &self,
    context: &RuntimeModuleGenerateContext<'_>,
  ) -> rspack_error::Result<String> {
    let export_name = if context.runtime_template.uses_runtime_context() {
      context
        .runtime_template
        .render_runtime_variable(&RuntimeVariable::Context)
    } else {
      context
        .runtime_template
        .render_runtime_variable(&RuntimeVariable::Require)
    };
    let export_temp_name = format!("{export_name}temp");
    Ok(format!(
      r#"var {export_temp_name} = {export_name};
export {{ {export_temp_name} as {export_name} }};
"#,
    ))
  }

  fn should_isolate(&self) -> bool {
    false
  }
}
