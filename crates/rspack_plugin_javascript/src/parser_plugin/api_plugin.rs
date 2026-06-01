use rspack_core::{ConstDependency, ModuleArgument, RuntimeGlobals, RuntimeRequirementsDependency};
use rspack_error::{Error, Severity};
use rspack_util::SpanExt;
use swc_core::{
  atoms::Atom,
  common::{Span, Spanned},
  ecma::ast::{CallExpr, Ident, Pat, UnaryExpr},
};

use crate::{
  dependency::{ModuleArgumentDependency, RequireMainDependency},
  parser_plugin::JavascriptParserPlugin,
  utils::eval::{self, BasicEvaluatedExpression},
  visitors::{
    IdentifierHookName, JavascriptParser, ParserHookName, Statement, VariableDeclaration,
    create_traceable_error,
  },
};

fn expression_not_supported(
  source: &str,
  name: &str,
  is_call: bool,
  expr_span: Span,
) -> (Error, Box<ConstDependency>) {
  let mut error = create_traceable_error(
    "Unsupported feature".into(),
    format!(
      "{name}{} is not supported by Rspack.",
      if is_call { "()" } else { "" }
    ),
    source.to_owned(),
    expr_span.into(),
  );
  error.severity = Severity::Warning;
  error.hide_stack = Some(true);
  (
    error,
    Box::new(ConstDependency::new(expr_span.into(), "(void 0)".into())),
  )
}

const API_HASH: &str = "__webpack_hash__";
const API_LAYER: &str = "__webpack_layer__";
const API_PUBLIC_PATH: &str = "__webpack_public_path__";
const API_MODULES: &str = "__webpack_modules__";
const API_MODULE: &str = "__webpack_module__";
const API_CHUNK_LOAD: &str = "__webpack_chunk_load__";
const API_BASE_URI: &str = "__webpack_base_uri__";
const API_NON_REQUIRE: &str = "__non_webpack_require__";
const API_SYSTEM_CONTEXT: &str = "__system_context__";
const API_SHARE_SCOPES: &str = "__webpack_share_scopes__";
const API_INIT_SHARING: &str = "__webpack_init_sharing__";
const API_NONCE: &str = "__webpack_nonce__";
const API_CHUNK_NAME: &str = "__webpack_chunkname__";
const API_RUNTIME_ID: &str = "__webpack_runtime_id__";
const API_REQUIRE: &str = "__webpack_require__";
const API_GET_SCRIPT_FILENAME: &str = "__webpack_get_script_filename__";
const API_VERSION: &str = "__rspack_version__";
const API_UNIQUE_ID: &str = "__rspack_unique_id__";
const API_RSC_MANIFEST: &str = "__rspack_rsc_manifest__";

thread_local! {
  static API_ATOMS: ApiAtoms = ApiAtoms::new();
}

struct ApiAtoms {
  hash: Atom,
  layer: Atom,
  public_path: Atom,
  modules: Atom,
  module: Atom,
  chunk_load: Atom,
  base_uri: Atom,
  non_require: Atom,
  system_context: Atom,
  share_scopes: Atom,
  init_sharing: Atom,
  nonce: Atom,
  chunk_name: Atom,
  runtime_id: Atom,
  require: Atom,
  get_script_filename: Atom,
  version: Atom,
  unique_id: Atom,
  rsc_manifest: Atom,
}

impl ApiAtoms {
  fn new() -> Self {
    Self {
      hash: Atom::from(API_HASH),
      layer: Atom::from(API_LAYER),
      public_path: Atom::from(API_PUBLIC_PATH),
      modules: Atom::from(API_MODULES),
      module: Atom::from(API_MODULE),
      chunk_load: Atom::from(API_CHUNK_LOAD),
      base_uri: Atom::from(API_BASE_URI),
      non_require: Atom::from(API_NON_REQUIRE),
      system_context: Atom::from(API_SYSTEM_CONTEXT),
      share_scopes: Atom::from(API_SHARE_SCOPES),
      init_sharing: Atom::from(API_INIT_SHARING),
      nonce: Atom::from(API_NONCE),
      chunk_name: Atom::from(API_CHUNK_NAME),
      runtime_id: Atom::from(API_RUNTIME_ID),
      require: Atom::from(API_REQUIRE),
      get_script_filename: Atom::from(API_GET_SCRIPT_FILENAME),
      version: Atom::from(API_VERSION),
      unique_id: Atom::from(API_UNIQUE_ID),
      rsc_manifest: Atom::from(API_RSC_MANIFEST),
    }
  }
}

#[derive(Clone, Copy)]
enum ApiIdentifier {
  Hash,
  Layer,
  PublicPath,
  Modules,
  Module,
  ChunkLoad,
  BaseUri,
  NonRequire,
  SystemContext,
  ShareScopes,
  InitSharing,
  Nonce,
  ChunkName,
  RuntimeId,
  Require,
  GetScriptFilename,
  Version,
  UniqueId,
  RscManifest,
}

#[inline]
fn get_api_identifier(name: &Atom) -> Option<ApiIdentifier> {
  API_ATOMS.with(|atoms| {
    if name == &atoms.hash {
      Some(ApiIdentifier::Hash)
    } else if name == &atoms.layer {
      Some(ApiIdentifier::Layer)
    } else if name == &atoms.nonce {
      Some(ApiIdentifier::Nonce)
    } else if name == &atoms.module {
      Some(ApiIdentifier::Module)
    } else if name == &atoms.system_context {
      Some(ApiIdentifier::SystemContext)
    } else if name == &atoms.version {
      Some(ApiIdentifier::Version)
    } else if name == &atoms.require {
      Some(ApiIdentifier::Require)
    } else if name == &atoms.modules {
      Some(ApiIdentifier::Modules)
    } else if name == &atoms.base_uri {
      Some(ApiIdentifier::BaseUri)
    } else if name == &atoms.unique_id {
      Some(ApiIdentifier::UniqueId)
    } else if name == &atoms.chunk_name {
      Some(ApiIdentifier::ChunkName)
    } else if name == &atoms.chunk_load {
      Some(ApiIdentifier::ChunkLoad)
    } else if name == &atoms.runtime_id {
      Some(ApiIdentifier::RuntimeId)
    } else if name == &atoms.public_path {
      Some(ApiIdentifier::PublicPath)
    } else if name == &atoms.non_require {
      Some(ApiIdentifier::NonRequire)
    } else if name == &atoms.rsc_manifest {
      Some(ApiIdentifier::RscManifest)
    } else if name == &atoms.share_scopes {
      Some(ApiIdentifier::ShareScopes)
    } else if name == &atoms.init_sharing {
      Some(ApiIdentifier::InitSharing)
    } else if name == &atoms.get_script_filename {
      Some(ApiIdentifier::GetScriptFilename)
    } else {
      None
    }
  })
}

pub struct APIPluginOptions {
  module: bool,
}

pub struct APIPlugin {
  options: APIPluginOptions,
}

impl APIPlugin {
  pub fn new(module: bool) -> Self {
    let options = APIPluginOptions { module };
    Self { options }
  }
}

fn get_typeof_evaluate_of_api(identifier: ApiIdentifier) -> Option<&'static str> {
  match identifier {
    ApiIdentifier::Require | ApiIdentifier::ChunkLoad => Some("function"),
    ApiIdentifier::Hash
    | ApiIdentifier::PublicPath
    | ApiIdentifier::BaseUri
    | ApiIdentifier::Nonce
    | ApiIdentifier::ChunkName
    | ApiIdentifier::Version
    | ApiIdentifier::UniqueId => Some("string"),
    ApiIdentifier::Modules
    | ApiIdentifier::Module
    | ApiIdentifier::SystemContext
    | ApiIdentifier::ShareScopes
    | ApiIdentifier::RscManifest => Some("object"),
    ApiIdentifier::InitSharing | ApiIdentifier::GetScriptFilename => Some("function"),
    ApiIdentifier::Layer | ApiIdentifier::NonRequire | ApiIdentifier::RuntimeId => None,
  }
}

#[rspack_macros::implemented_javascript_parser_hooks]
impl JavascriptParserPlugin for APIPlugin {
  fn evaluate_typeof<'a>(
    &self,
    parser: &mut JavascriptParser,
    expr: &'a UnaryExpr,
    for_name: ParserHookName<'_>,
  ) -> Option<BasicEvaluatedExpression<'a>> {
    let identifier = get_api_identifier(for_name.as_atom()?)?;
    if matches!(identifier, ApiIdentifier::Layer) {
      let value = if parser.module_layer.is_none() {
        "object"
      } else {
        "string"
      };
      return Some(eval::evaluate_to_string(
        value.to_string(),
        expr.span.real_lo(),
        expr.span.real_hi(),
      ));
    }

    get_typeof_evaluate_of_api(identifier).map(|res| {
      eval::evaluate_to_string(res.to_string(), expr.span.real_lo(), expr.span.real_hi())
    })
  }

  fn identifier(
    &self,
    parser: &mut JavascriptParser,
    ident: &Ident,
    for_name: IdentifierHookName<'_>,
  ) -> Option<bool> {
    let identifier = get_api_identifier(for_name.as_atom()?)?;

    if matches!(identifier, ApiIdentifier::Layer) {
      parser.add_presentational_dependency(Box::new(ConstDependency::new(
        ident.span.into(),
        serde_json::to_string(&parser.module_layer)
          .expect("should stringify JSON")
          .into(),
      )));
      return Some(true);
    }

    if matches!(identifier, ApiIdentifier::Require) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::REQUIRE,
      )));
      Some(true)
    } else if matches!(identifier, ApiIdentifier::Hash) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::call(
        ident.span.into(),
        RuntimeGlobals::GET_FULL_HASH,
      )));
      Some(true)
    } else if matches!(identifier, ApiIdentifier::PublicPath) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::PUBLIC_PATH,
      )));
      Some(true)
    } else if matches!(identifier, ApiIdentifier::Modules) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::MODULE_FACTORIES,
      )));
      Some(true)
    } else if matches!(identifier, ApiIdentifier::ChunkLoad) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::ENSURE_CHUNK,
      )));
      Some(true)
    } else if matches!(identifier, ApiIdentifier::Module) {
      let range = ident.span.into();
      let loc = parser.to_dependency_location(range);
      parser
        .add_presentational_dependency(Box::new(ModuleArgumentDependency::new(None, range, loc)));
      Some(true)
    } else if matches!(identifier, ApiIdentifier::BaseUri) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::BASE_URI,
      )));
      Some(true)
    } else if matches!(identifier, ApiIdentifier::NonRequire) {
      let content = if self.options.module {
        parser.build_info.need_create_require = true;
        "__rspack_createRequire_require".into()
      } else {
        "require".into()
      };
      parser
        .add_presentational_dependency(Box::new(ConstDependency::new(ident.span.into(), content)));
      Some(true)
    } else if matches!(identifier, ApiIdentifier::SystemContext) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::SYSTEM_CONTEXT,
      )));
      Some(true)
    } else if matches!(identifier, ApiIdentifier::ShareScopes) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::SHARE_SCOPE_MAP,
      )));
      Some(true)
    } else if matches!(identifier, ApiIdentifier::InitSharing) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::INITIALIZE_SHARING,
      )));
      Some(true)
    } else if matches!(identifier, ApiIdentifier::Nonce) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::SCRIPT_NONCE,
      )));
      Some(true)
    } else if matches!(identifier, ApiIdentifier::ChunkName) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::CHUNK_NAME,
      )));
      Some(true)
    } else if matches!(identifier, ApiIdentifier::RuntimeId) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::RUNTIME_ID,
      )));
      Some(true)
    } else if matches!(identifier, ApiIdentifier::GetScriptFilename) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::GET_CHUNK_SCRIPT_FILENAME,
      )));
      Some(true)
    } else if matches!(identifier, ApiIdentifier::Version) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::call(
        ident.span.into(),
        RuntimeGlobals::RSPACK_VERSION,
      )));
      Some(true)
    } else if matches!(identifier, ApiIdentifier::UniqueId) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::RSPACK_UNIQUE_ID,
      )));
      Some(true)
    } else if matches!(identifier, ApiIdentifier::RscManifest) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::RSC_MANIFEST,
      )));
      Some(true)
    } else {
      None
    }
  }

  fn evaluate_identifier(
    &self,
    parser: &mut JavascriptParser,
    for_name: ParserHookName<'_>,
    start: u32,
    end: u32,
  ) -> Option<eval::BasicEvaluatedExpression<'static>> {
    if matches!(
      for_name.as_atom().and_then(get_api_identifier),
      Some(ApiIdentifier::Layer)
    ) {
      if let Some(layer) = parser.module_layer {
        Some(eval::evaluate_to_string(layer.into(), start, end))
      } else {
        Some(eval::evaluate_to_null(start, end))
      }
    } else {
      None
    }
  }

  fn member(
    &self,
    parser: &mut JavascriptParser,
    member_expr: &swc_core::ecma::ast::MemberExpr,
    for_name: &str,
  ) -> Option<bool> {
    if for_name == "require.extensions"
      || for_name == "require.config"
      || for_name == "require.version"
      || for_name == "require.include"
      || for_name == "require.onError"
      || for_name == "require.main.require"
      || for_name == "module.parent.require"
    {
      let (warning, dep) =
        expression_not_supported(parser.source, for_name, false, member_expr.span());
      parser.add_warning(warning.into());
      parser.add_presentational_dependency(dep);
      return Some(true);
    }

    if for_name == "require.cache" {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        member_expr.span().into(),
        RuntimeGlobals::MODULE_CACHE,
      )));
      return Some(true);
    }

    if for_name == "require.main" {
      parser.add_presentational_dependency(Box::new(RequireMainDependency::new(
        member_expr.span().into(),
      )));
      return Some(true);
    }

    if for_name == "__webpack_module__.id" {
      let range = member_expr.span.into();
      let loc = parser.to_dependency_location(range);
      parser.add_presentational_dependency(Box::new(ModuleArgumentDependency::new(
        Some("id".into()),
        range,
        loc,
      )));
      return Some(true);
    }

    None
  }

  fn pre_declarator(
    &self,
    parser: &mut JavascriptParser,
    declarator: &swc_core::ecma::ast::VarDeclarator,
    _declaration: VariableDeclaration<'_>,
  ) -> Option<bool> {
    // Check if we're at top level scope and the declarator is a simple identifier named "module"
    if parser.is_top_level_scope()
      && let Pat::Ident(ident) = &declarator.name
      && ident.id.sym.as_ref() == "module"
    {
      parser.build_info.module_argument = ModuleArgument::RspackModule;
    }
    None
  }

  fn pre_statement(&self, parser: &mut JavascriptParser, stmt: Statement) -> Option<bool> {
    // Check if we're at top level scope
    if parser.is_top_level_scope() {
      match stmt {
        Statement::Fn(fn_decl) => {
          // Check for function declaration named "module"
          if let Some(ident) = fn_decl.ident()
            && ident.sym.as_ref() == "module"
          {
            parser.build_info.module_argument = ModuleArgument::RspackModule;
          }
        }
        Statement::Class(class_decl) => {
          // Check for class declaration named "module"
          if let Some(ident) = class_decl.ident()
            && ident.sym.as_ref() == "module"
          {
            parser.build_info.module_argument = ModuleArgument::RspackModule;
          }
        }
        _ => {}
      }
    }
    None
  }

  fn call(
    &self,
    parser: &mut JavascriptParser,
    call_expr: &CallExpr,
    for_name: &str,
  ) -> Option<bool> {
    if for_name == "require.config"
      || for_name == "require.include"
      || for_name == "require.onError"
      || for_name == "require.main.require"
      || for_name == "module.parent.require"
    {
      let (warning, dep) =
        expression_not_supported(parser.source, for_name, true, call_expr.span());
      parser.add_warning(warning.into());
      parser.add_presentational_dependency(dep);
      return Some(true);
    }

    None
  }
}
