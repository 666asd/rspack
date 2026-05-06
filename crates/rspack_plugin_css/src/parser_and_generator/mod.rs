pub mod generate;
pub mod impl_parser_and_generator;

use std::sync::LazyLock;

use regex::Regex;
use rspack_cacheable::cacheable;
pub use rspack_core::{CssExport, CssExports};
use rspack_core::{
  CssExportType, CssExportsConvention, CssModuleGeneratorOptions, CssModuleParserOptions,
  CssParserImport, Dependency, ExportsInfoArtifact, LocalIdentName, Module, ModuleIdentifier,
  ParserAndGenerator, RuntimeSpec, SourceType, UsageState,
  rspack_sources::{Source, SourceExt},
};
use rspack_error::IntoTWithDiagnosticArray;
use rspack_util::{
  atom::Atom,
  ext::DynHash,
  fx_hash::{FxIndexMap, FxIndexSet},
};
use rustc_hash::{FxHashMap, FxHashSet};

static REGEX_IS_MODULES: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"(?i)\.modules?\.[^.]+$").expect("Invalid regex"));

static REGEX_IS_COMMENTS: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"/\*[\s\S]*?\*/").expect("Invalid regex"));

pub(crate) static CSS_MODULE_SOURCE_TYPE_LIST: &[SourceType; 1] = &[SourceType::Css];

pub(crate) static CSS_MODULE_AND_JS_SOURCE_TYPE_LIST: &[SourceType; 2] =
  &[SourceType::Css, SourceType::JavaScript];

pub(crate) static CSS_MODULE_EXPORTS_ONLY_SOURCE_TYPE_LIST: &[SourceType; 1] =
  &[SourceType::JavaScript];

#[cacheable]
#[derive(Debug)]
pub struct CssParserAndGenerator {
  pub generator_options: CssModuleGeneratorOptions,
  pub parser_options: CssModuleParserOptions,
  pub hot: bool,
}

impl CssParserAndGenerator {
  pub fn new(
    generator_options: CssModuleGeneratorOptions,
    parser_options: CssModuleParserOptions,
  ) -> Self {
    Self {
      generator_options,
      parser_options,
      hot: false,
    }
  }

  pub fn convention(&self) -> &CssExportsConvention {
    self
      .generator_options
      .exports_convention
      .as_ref()
      .expect("should have convention for module_type css/auto or css/module")
  }

  pub fn local_ident_name(&self) -> &LocalIdentName {
    self
      .generator_options
      .local_ident_name
      .as_ref()
      .expect("should have local_ident_name for module_type css/auto or css/module")
  }

  pub fn exports_only(&self) -> bool {
    self
      .generator_options
      .exports_only
      .expect("should have exports_only")
  }

  pub fn named_exports(&self) -> bool {
    self.parser_options.named_exports.unwrap_or(true)
  }

  pub fn es_module(&self) -> bool {
    self
      .generator_options
      .es_module
      .expect("should have es_module")
  }

  pub fn resolve_import(&self) -> &CssParserImport {
    self
      .parser_options
      .resolve_import
      .as_ref()
      .unwrap_or(&CssParserImport::Bool(true))
  }

  pub fn url(&self) -> bool {
    self.parser_options.url.unwrap_or(true)
  }

  pub fn animation(&self) -> Option<bool> {
    self.parser_options.animation
  }

  pub fn container(&self) -> Option<bool> {
    self.parser_options.container
  }

  pub fn custom_idents(&self) -> Option<bool> {
    self.parser_options.custom_idents
  }

  pub fn dashed_idents(&self) -> Option<bool> {
    self.parser_options.dashed_idents
  }

  pub fn function(&self) -> Option<bool> {
    self.parser_options.function
  }

  pub fn grid(&self) -> Option<bool> {
    self.parser_options.grid
  }

  pub fn export_type(&self) -> &Option<CssExportType> {
    &self.parser_options.export_type
  }
}

pub fn get_used_exports<'a>(
  exports: &'a CssExports,
  identifier: ModuleIdentifier,
  runtime: Option<&RuntimeSpec>,
  exports_info_artifact: &ExportsInfoArtifact,
) -> FxIndexMap<&'a str, &'a FxIndexSet<CssExport>> {
  let exports_info = exports_info_artifact
    .get_exports_info_optional(&identifier)
    .map(|info| info.as_data(exports_info_artifact));

  exports
    .iter()
    .filter(|(name, _)| {
      let export_info = exports_info
        .as_ref()
        .map(|info| info.get_read_only_export_info(&Atom::from(name.as_str())));

      if let Some(export_info) = export_info {
        export_info.get_used(runtime) != UsageState::Unused
      } else {
        true
      }
    })
    .map(|(name, exports)| (name.as_str(), exports))
    .collect()
}

#[derive(Debug, Clone)]
pub struct CodeGenerationDataUnusedLocalIdent {
  pub(crate) idents: FxHashSet<String>,
}

pub fn get_unused_local_ident(
  exports: &CssExports,
  local_names: &FxHashMap<String, String>,
  identifier: ModuleIdentifier,
  runtime: Option<&RuntimeSpec>,
  exports_info_artifact: &ExportsInfoArtifact,
) -> CodeGenerationDataUnusedLocalIdent {
  let exports_names = exports.iter().fold(
    FxHashMap::<&str, FxHashSet<Atom>>::default(),
    |mut map, (name, css_exports)| {
      css_exports.iter().for_each(|css_export| {
        if let Some(set) = map.get_mut(css_export.orig_name.as_str()) {
          set.insert(Atom::from(name.clone()));
        } else {
          map.insert(
            &css_export.orig_name,
            FxHashSet::from_iter([Atom::from(name.clone())]),
          );
        }
      });
      map
    },
  );

  let exports_info = exports_info_artifact
    .get_exports_info_optional(&identifier)
    .map(|info| info.as_data(exports_info_artifact));

  CodeGenerationDataUnusedLocalIdent {
    idents: exports_names
      .iter()
      .filter(|(_, export_names)| {
        export_names.iter().all(|export_name| {
          let export_info = exports_info
            .as_ref()
            .map(|info| info.get_read_only_export_info(export_name));

          if let Some(export_info) = export_info {
            matches!(export_info.get_used(runtime), UsageState::Unused)
          } else {
            false
          }
        })
      })
      .filter_map(|(css_name, _)| local_names.get(*css_name).cloned())
      .collect(),
  }
}
