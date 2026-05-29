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
    JavascriptParser, ParserHookName, Statement, VariableDeclaration, create_traceable_error,
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
  static API_HASH_ATOM: Atom = Atom::from(API_HASH);
  static API_LAYER_ATOM: Atom = Atom::from(API_LAYER);
  static API_PUBLIC_PATH_ATOM: Atom = Atom::from(API_PUBLIC_PATH);
  static API_MODULES_ATOM: Atom = Atom::from(API_MODULES);
  static API_MODULE_ATOM: Atom = Atom::from(API_MODULE);
  static API_CHUNK_LOAD_ATOM: Atom = Atom::from(API_CHUNK_LOAD);
  static API_BASE_URI_ATOM: Atom = Atom::from(API_BASE_URI);
  static API_NON_REQUIRE_ATOM: Atom = Atom::from(API_NON_REQUIRE);
  static API_SYSTEM_CONTEXT_ATOM: Atom = Atom::from(API_SYSTEM_CONTEXT);
  static API_SHARE_SCOPES_ATOM: Atom = Atom::from(API_SHARE_SCOPES);
  static API_INIT_SHARING_ATOM: Atom = Atom::from(API_INIT_SHARING);
  static API_NONCE_ATOM: Atom = Atom::from(API_NONCE);
  static API_CHUNK_NAME_ATOM: Atom = Atom::from(API_CHUNK_NAME);
  static API_RUNTIME_ID_ATOM: Atom = Atom::from(API_RUNTIME_ID);
  static API_REQUIRE_ATOM: Atom = Atom::from(API_REQUIRE);
  static API_GET_SCRIPT_FILENAME_ATOM: Atom = Atom::from(API_GET_SCRIPT_FILENAME);
  static API_VERSION_ATOM: Atom = Atom::from(API_VERSION);
  static API_UNIQUE_ID_ATOM: Atom = Atom::from(API_UNIQUE_ID);
  static API_RSC_MANIFEST_ATOM: Atom = Atom::from(API_RSC_MANIFEST);
}

macro_rules! is_api_identifier {
  ($name:expr, $atom:ident) => {
    $atom.with(|atom| $name.is_identifier(atom))
  };
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

fn get_typeof_evaluate_of_api(for_name: ParserHookName<'_>) -> Option<&'static str> {
  if is_api_identifier!(for_name, API_REQUIRE_ATOM) {
    Some("function")
  } else if is_api_identifier!(for_name, API_HASH_ATOM) {
    Some("string")
  } else if is_api_identifier!(for_name, API_PUBLIC_PATH_ATOM) {
    Some("string")
  } else if is_api_identifier!(for_name, API_MODULES_ATOM)
    || is_api_identifier!(for_name, API_MODULE_ATOM)
  {
    Some("object")
  } else if is_api_identifier!(for_name, API_CHUNK_LOAD_ATOM) {
    Some("function")
  } else if is_api_identifier!(for_name, API_BASE_URI_ATOM) {
    Some("string")
  } else if is_api_identifier!(for_name, API_SYSTEM_CONTEXT_ATOM)
    || is_api_identifier!(for_name, API_SHARE_SCOPES_ATOM)
    || is_api_identifier!(for_name, API_RSC_MANIFEST_ATOM)
  {
    Some("object")
  } else if is_api_identifier!(for_name, API_INIT_SHARING_ATOM)
    || is_api_identifier!(for_name, API_GET_SCRIPT_FILENAME_ATOM)
  {
    Some("function")
  } else if is_api_identifier!(for_name, API_NONCE_ATOM)
    || is_api_identifier!(for_name, API_CHUNK_NAME_ATOM)
    || is_api_identifier!(for_name, API_VERSION_ATOM)
    || is_api_identifier!(for_name, API_UNIQUE_ID_ATOM)
  {
    Some("string")
  } else {
    None
  }
}

#[inline]
fn is_api_layer(name: ParserHookName<'_>) -> bool {
  is_api_identifier!(name, API_LAYER_ATOM)
}

#[rspack_macros::implemented_javascript_parser_hooks]
impl JavascriptParserPlugin for APIPlugin {
  fn evaluate_typeof<'a>(
    &self,
    parser: &mut JavascriptParser,
    expr: &'a UnaryExpr,
    for_name: ParserHookName<'_>,
  ) -> Option<BasicEvaluatedExpression<'a>> {
    if is_api_layer(for_name) {
      let value = if parser.module_layer.is_none() {
        "object"
      } else {
        "string"
      };
      Some(eval::evaluate_to_string(
        value.to_string(),
        expr.span.real_lo(),
        expr.span.real_hi(),
      ))
    } else {
      get_typeof_evaluate_of_api(for_name).map(|res| {
        eval::evaluate_to_string(res.to_string(), expr.span.real_lo(), expr.span.real_hi())
      })
    }
  }

  fn identifier(
    &self,
    parser: &mut JavascriptParser,
    ident: &Ident,
    for_name: ParserHookName<'_>,
  ) -> Option<bool> {
    if is_api_layer(for_name) {
      parser.add_presentational_dependency(Box::new(ConstDependency::new(
        ident.span.into(),
        serde_json::to_string(&parser.module_layer)
          .expect("should stringify JSON")
          .into(),
      )));
      return Some(true);
    }

    if is_api_identifier!(for_name, API_REQUIRE_ATOM) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::REQUIRE,
      )));
      Some(true)
    } else if is_api_identifier!(for_name, API_HASH_ATOM) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::call(
        ident.span.into(),
        RuntimeGlobals::GET_FULL_HASH,
      )));
      Some(true)
    } else if is_api_identifier!(for_name, API_PUBLIC_PATH_ATOM) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::PUBLIC_PATH,
      )));
      Some(true)
    } else if is_api_identifier!(for_name, API_MODULES_ATOM) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::MODULE_FACTORIES,
      )));
      Some(true)
    } else if is_api_identifier!(for_name, API_CHUNK_LOAD_ATOM) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::ENSURE_CHUNK,
      )));
      Some(true)
    } else if is_api_identifier!(for_name, API_MODULE_ATOM) {
      let range = ident.span.into();
      let loc = parser.to_dependency_location(range);
      parser
        .add_presentational_dependency(Box::new(ModuleArgumentDependency::new(None, range, loc)));
      Some(true)
    } else if is_api_identifier!(for_name, API_BASE_URI_ATOM) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::BASE_URI,
      )));
      Some(true)
    } else if is_api_identifier!(for_name, API_NON_REQUIRE_ATOM) {
      let content = if self.options.module {
        parser.build_info.need_create_require = true;
        "__rspack_createRequire_require".into()
      } else {
        "require".into()
      };
      parser
        .add_presentational_dependency(Box::new(ConstDependency::new(ident.span.into(), content)));
      Some(true)
    } else if is_api_identifier!(for_name, API_SYSTEM_CONTEXT_ATOM) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::SYSTEM_CONTEXT,
      )));
      Some(true)
    } else if is_api_identifier!(for_name, API_SHARE_SCOPES_ATOM) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::SHARE_SCOPE_MAP,
      )));
      Some(true)
    } else if is_api_identifier!(for_name, API_INIT_SHARING_ATOM) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::INITIALIZE_SHARING,
      )));
      Some(true)
    } else if is_api_identifier!(for_name, API_NONCE_ATOM) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::SCRIPT_NONCE,
      )));
      Some(true)
    } else if is_api_identifier!(for_name, API_CHUNK_NAME_ATOM) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::CHUNK_NAME,
      )));
      Some(true)
    } else if is_api_identifier!(for_name, API_RUNTIME_ID_ATOM) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::RUNTIME_ID,
      )));
      Some(true)
    } else if is_api_identifier!(for_name, API_GET_SCRIPT_FILENAME_ATOM) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::GET_CHUNK_SCRIPT_FILENAME,
      )));
      Some(true)
    } else if is_api_identifier!(for_name, API_VERSION_ATOM) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::call(
        ident.span.into(),
        RuntimeGlobals::RSPACK_VERSION,
      )));
      Some(true)
    } else if is_api_identifier!(for_name, API_UNIQUE_ID_ATOM) {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        ident.span.into(),
        RuntimeGlobals::RSPACK_UNIQUE_ID,
      )));
      Some(true)
    } else if is_api_identifier!(for_name, API_RSC_MANIFEST_ATOM) {
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
    if is_api_layer(for_name) {
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
