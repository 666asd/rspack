use cow_utils::CowUtils;
use rspack_cacheable::{cacheable, cacheable_dyn};
use rspack_core::{
  AsContextDependency, CodeGenerationDataAssetInfo, CodeGenerationDataFilename,
  CodeGenerationDataRealContentHash, CodeGenerationDataUrl, Compilation, Dependency,
  DependencyCategory, DependencyCodeGeneration, DependencyId, DependencyRange, DependencyTemplate,
  DependencyTemplateType, DependencyType, FactorizeInfo, ModuleDependency, ModuleIdentifier,
  TemplateContext, TemplateReplaceSource, record_source_content_hash_references,
};

use crate::utils::{AUTO_PUBLIC_PATH_PLACEHOLDER, css_escape_string};

#[cacheable]
#[derive(Debug, Clone)]
pub struct CssUrlDependency {
  id: DependencyId,
  request: String,
  range: DependencyRange,
  replace_function: bool,
  factorize_info: FactorizeInfo,
}

impl CssUrlDependency {
  pub fn new(request: String, range: DependencyRange, replace_function: bool) -> Self {
    Self {
      request,
      range,
      id: DependencyId::new(),
      replace_function,
      factorize_info: Default::default(),
    }
  }

  fn get_target_url(
    &self,
    identifier: &ModuleIdentifier,
    compilation: &Compilation,
  ) -> Option<String> {
    // url points to asset modules, and asset modules should have same codegen results for all runtimes
    let code_gen_result = compilation.code_generation_results.get_one(identifier);

    if let Some(url) = code_gen_result.data.get::<CodeGenerationDataUrl>() {
      Some(url.inner().to_string())
    } else if let Some(data) = code_gen_result.data.get::<CodeGenerationDataFilename>() {
      let filename = data.filename();
      let public_path = data.public_path().cow_replace(
        "__RSPACK_PLUGIN_ASSET_AUTO_PUBLIC_PATH__",
        AUTO_PUBLIC_PATH_PLACEHOLDER,
      );
      Some(format!("{public_path}{filename}"))
    } else {
      None
    }
  }
}

#[cacheable_dyn]
impl Dependency for CssUrlDependency {
  fn id(&self) -> &DependencyId {
    &self.id
  }

  fn category(&self) -> &DependencyCategory {
    &DependencyCategory::Url
  }

  fn dependency_type(&self) -> &DependencyType {
    &DependencyType::CssUrl
  }

  fn range(&self) -> Option<DependencyRange> {
    Some(self.range)
  }

  fn could_affect_referencing_module(&self) -> rspack_core::AffectType {
    rspack_core::AffectType::True
  }
}

#[cacheable_dyn]
impl ModuleDependency for CssUrlDependency {
  fn request(&self) -> &str {
    &self.request
  }

  fn user_request(&self) -> &str {
    &self.request
  }

  fn factorize_info(&self) -> &FactorizeInfo {
    &self.factorize_info
  }

  fn factorize_info_mut(&mut self) -> &mut FactorizeInfo {
    &mut self.factorize_info
  }
}

#[cacheable_dyn]
impl DependencyCodeGeneration for CssUrlDependency {
  fn dependency_template(&self) -> Option<DependencyTemplateType> {
    Some(CssUrlDependencyTemplate::template_type())
  }
}

impl AsContextDependency for CssUrlDependency {}

#[cacheable]
#[derive(Debug, Clone, Default)]
pub struct CssUrlDependencyTemplate;

#[derive(Clone, Debug, Default)]
struct CodeGenerationDataCssUrlReplacementOffset {
  offset: i64,
}

impl CssUrlDependencyTemplate {
  pub fn template_type() -> DependencyTemplateType {
    DependencyTemplateType::Dependency(DependencyType::CssUrl)
  }
}

impl DependencyTemplate for CssUrlDependencyTemplate {
  fn render(
    &self,
    dep: &dyn DependencyCodeGeneration,
    source: &mut TemplateReplaceSource,
    code_generatable_context: &mut TemplateContext,
  ) {
    let dep = dep
      .as_any()
      .downcast_ref::<CssUrlDependency>()
      .expect("CssUrlDependencyTemplate should be used for CssUrlDependency");

    let TemplateContext { compilation, .. } = code_generatable_context;
    if let Some(mgm) = compilation
      .get_module_graph()
      .module_graph_module_by_dependency_id(dep.id())
      && let Some(target_url) = dep.get_target_url(&mgm.module_identifier, compilation)
    {
      let target_url = css_escape_string(&target_url);
      let content = if dep.replace_function {
        format!("url({target_url})")
      } else {
        target_url
      };
      if compilation.options.optimization.real_content_hash {
        let offset = code_generatable_context
          .data
          .get::<CodeGenerationDataCssUrlReplacementOffset>()
          .map(|data| data.offset)
          .unwrap_or_default();
        let replacement_start = i64::from(dep.range.start)
          .checked_add(offset)
          .expect("CSS url replacement offset should fit in i64");
        let code_gen_result = compilation
          .code_generation_results
          .get_one(&mgm.module_identifier);
        if let Some(asset_info) = code_gen_result.data.get::<CodeGenerationDataAssetInfo>()
          && let Ok(replacement_start) = u32::try_from(replacement_start)
        {
          let mut real_content_hashes = code_generatable_context
            .data
            .get::<CodeGenerationDataRealContentHash>()
            .cloned()
            .unwrap_or_default();
          record_source_content_hash_references(
            real_content_hashes.inner_mut(),
            &content,
            replacement_start,
            asset_info.inner().content_hash.iter(),
          );
          code_generatable_context.data.insert(real_content_hashes);
        }
        let original_len = i64::from(dep.range.end)
          .checked_sub(i64::from(dep.range.start))
          .expect("CSS url replacement range length should fit in i64");
        let offset = offset
          .checked_add(
            i64::try_from(content.len()).expect("CSS url replacement length should fit in i64")
              - original_len,
          )
          .expect("CSS url replacement offset should fit in i64");
        code_generatable_context
          .data
          .insert(CodeGenerationDataCssUrlReplacementOffset { offset });
      }
      source.replace(dep.range.start, dep.range.end, content, None);
    }
  }
}
