use rspack_core::{
  RuntimeGlobals, RuntimeModule, RuntimeModuleGenerateContext, RuntimeModuleStage, RuntimeTemplate,
  impl_runtime_module,
};

#[impl_runtime_module]
#[derive(Debug)]
pub struct DefinePropertyGettersCompatRuntimeModule {}

impl DefinePropertyGettersCompatRuntimeModule {
  pub fn new(runtime_template: &RuntimeTemplate) -> Self {
    Self::with_default(runtime_template)
  }
}

#[async_trait::async_trait]
impl RuntimeModule for DefinePropertyGettersCompatRuntimeModule {
  fn stage(&self) -> RuntimeModuleStage {
    RuntimeModuleStage::Attach
  }

  async fn generate(
    &self,
    context: &RuntimeModuleGenerateContext<'_>,
  ) -> rspack_error::Result<String> {
    let require = context
      .runtime_template
      .render_runtime_globals(&RuntimeGlobals::REQUIRE);
    let define_property_getters = context
      .runtime_template
      .render_runtime_globals(&RuntimeGlobals::DEFINE_PROPERTY_GETTERS);

    Ok(format!(
      r#"if(!{require}.rstest_define_property_getters) {{
	{require}.rstest_define_property_getters = {define_property_getters};
	{define_property_getters} = function(exports, definition) {{
		// Rstest injected runtime code may still call d(exports, {{ key: getter }}),
		// while Rspack's optimized runtime expects d(exports, [key, getter]).
		if(definition && typeof definition === "object" && !Array.isArray(definition)) {{
			var normalizedDefinition = [];
			for(var key in definition) {{
				if({require}.o(definition, key)) {{
					normalizedDefinition.push(key, definition[key]);
				}}
			}}
			definition = normalizedDefinition;
		}}
		return {require}.rstest_define_property_getters(exports, definition);
	}};
}}"#
    ))
  }

  fn additional_runtime_requirements(
    &self,
    _compilation: &rspack_core::Compilation,
  ) -> RuntimeGlobals {
    RuntimeGlobals::REQUIRE
      .union(RuntimeGlobals::DEFINE_PROPERTY_GETTERS)
      .union(RuntimeGlobals::HAS_OWN_PROPERTY)
  }
}
