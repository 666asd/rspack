use rspack_core::{
  ConstDependency, ModuleArgument, RuntimeGlobals, RuntimeRequirementsDependency, RuntimeVariable,
  runtime_globals_from_property_name, runtime_variable_to_string,
};
use rspack_error::{Error, Severity};
use rspack_util::SpanExt;
use swc_core::{
  atoms::Atom,
  common::{Span, Spanned},
  ecma::ast::{CallExpr, Ident, MemberExpr, MemberProp, Pat, UnaryExpr},
};

use crate::{
  dependency::{ModuleArgumentDependency, RequireMainDependency},
  parser_plugin::JavascriptParserPlugin,
  utils::eval::{self, BasicEvaluatedExpression},
  visitors::{
    ExportedVariableInfo, JavascriptParser, Statement, VariableDeclaration, create_traceable_error,
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

fn is_runtime_proxy_name(parser: &JavascriptParser, for_name: &str) -> bool {
  parser
    .compiler_options
    .experiments
    .runtime_mode
    .is_runtime_requirements_proxy_enabled()
    && for_name
      == runtime_variable_to_string(&RuntimeVariable::Runtime, parser.compiler_options).as_str()
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

fn get_typeof_evaluate_of_api(sym: &str) -> Option<&str> {
  match sym {
    API_REQUIRE => Some("function"),
    API_HASH => Some("string"),
    API_PUBLIC_PATH => Some("string"),
    API_MODULES => Some("object"),
    API_MODULE => Some("object"),
    API_CHUNK_LOAD => Some("function"),
    API_BASE_URI => Some("string"),
    API_NON_REQUIRE => None,
    API_SYSTEM_CONTEXT => Some("object"),
    API_SHARE_SCOPES => Some("object"),
    API_INIT_SHARING => Some("function"),
    API_NONCE => Some("string"),
    API_CHUNK_NAME => Some("string"),
    API_RUNTIME_ID => None,
    API_GET_SCRIPT_FILENAME => Some("function"),
    API_VERSION => Some("string"),
    API_UNIQUE_ID => Some("string"),
    API_RSC_MANIFEST => Some("object"),
    _ => None,
  }
}

fn api_runtime_global(sym: &str) -> Option<RuntimeGlobals> {
  Some(match sym {
    API_HASH => RuntimeGlobals::GET_FULL_HASH,
    API_PUBLIC_PATH => RuntimeGlobals::PUBLIC_PATH,
    API_MODULES => RuntimeGlobals::MODULE_FACTORIES,
    API_CHUNK_LOAD => RuntimeGlobals::ENSURE_CHUNK,
    API_BASE_URI => RuntimeGlobals::BASE_URI,
    API_SYSTEM_CONTEXT => RuntimeGlobals::SYSTEM_CONTEXT,
    API_SHARE_SCOPES => RuntimeGlobals::SHARE_SCOPE_MAP,
    API_INIT_SHARING => RuntimeGlobals::INITIALIZE_SHARING,
    API_NONCE => RuntimeGlobals::SCRIPT_NONCE,
    API_CHUNK_NAME => RuntimeGlobals::CHUNK_NAME,
    API_RUNTIME_ID => RuntimeGlobals::RUNTIME_ID,
    API_GET_SCRIPT_FILENAME => RuntimeGlobals::GET_CHUNK_SCRIPT_FILENAME,
    API_VERSION => RuntimeGlobals::RSPACK_VERSION,
    API_UNIQUE_ID => RuntimeGlobals::RSPACK_UNIQUE_ID,
    API_RSC_MANIFEST => RuntimeGlobals::RSC_MANIFEST,
    _ => return None,
  })
}

#[rspack_macros::implemented_javascript_parser_hooks]
impl JavascriptParserPlugin for APIPlugin {
  fn evaluate_typeof<'a>(
    &self,
    parser: &mut JavascriptParser,
    expr: &'a UnaryExpr,
    for_name: &str,
  ) -> Option<BasicEvaluatedExpression<'a>> {
    if for_name == API_LAYER {
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
    for_name: &str,
  ) -> Option<bool> {
    match for_name {
      API_REQUIRE => {
        if parser
          .compiler_options
          .experiments
          .runtime_mode
          .is_runtime_requirements_proxy_enabled()
        {
          parser.add_presentational_dependency(Box::new(
            RuntimeRequirementsDependency::require_bridge(),
          ));
        }
        parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
          ident.span.into(),
          RuntimeGlobals::REQUIRE,
        )));
        Some(true)
      }
      API_HASH => {
        parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::call(
          ident.span.into(),
          RuntimeGlobals::GET_FULL_HASH,
        )));
        Some(true)
      }
      API_LAYER => {
        parser.add_presentational_dependency(Box::new(ConstDependency::new(
          ident.span.into(),
          serde_json::to_string(&parser.module_layer)
            .expect("should stringify JSON")
            .into(),
        )));
        Some(true)
      }
      API_PUBLIC_PATH => {
        parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
          ident.span.into(),
          RuntimeGlobals::PUBLIC_PATH,
        )));
        Some(true)
      }
      API_MODULES => {
        parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
          ident.span.into(),
          RuntimeGlobals::MODULE_FACTORIES,
        )));
        Some(true)
      }
      API_CHUNK_LOAD => {
        parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
          ident.span.into(),
          RuntimeGlobals::ENSURE_CHUNK,
        )));
        Some(true)
      }
      API_MODULE => {
        let range = ident.span.into();
        let loc = parser.to_dependency_location(range);
        parser
          .add_presentational_dependency(Box::new(ModuleArgumentDependency::new(None, range, loc)));
        Some(true)
      }
      API_BASE_URI => {
        parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
          ident.span.into(),
          RuntimeGlobals::BASE_URI,
        )));
        Some(true)
      }
      API_NON_REQUIRE => {
        let content = if self.options.module {
          parser.build_info.need_create_require = true;
          "__rspack_createRequire_require".into()
        } else {
          "require".into()
        };
        parser.add_presentational_dependency(Box::new(ConstDependency::new(
          ident.span.into(),
          content,
        )));
        Some(true)
      }
      API_SYSTEM_CONTEXT => {
        parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
          ident.span.into(),
          RuntimeGlobals::SYSTEM_CONTEXT,
        )));
        Some(true)
      }
      API_SHARE_SCOPES => {
        parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
          ident.span.into(),
          RuntimeGlobals::SHARE_SCOPE_MAP,
        )));
        Some(true)
      }
      API_INIT_SHARING => {
        parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
          ident.span.into(),
          RuntimeGlobals::INITIALIZE_SHARING,
        )));
        Some(true)
      }
      API_NONCE => {
        parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
          ident.span.into(),
          RuntimeGlobals::SCRIPT_NONCE,
        )));
        Some(true)
      }
      API_CHUNK_NAME => {
        parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
          ident.span.into(),
          RuntimeGlobals::CHUNK_NAME,
        )));
        Some(true)
      }
      API_RUNTIME_ID => {
        parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
          ident.span.into(),
          RuntimeGlobals::RUNTIME_ID,
        )));
        Some(true)
      }
      API_GET_SCRIPT_FILENAME => {
        parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
          ident.span.into(),
          RuntimeGlobals::GET_CHUNK_SCRIPT_FILENAME,
        )));
        Some(true)
      }
      // rspack specific
      API_VERSION => {
        parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::call(
          ident.span.into(),
          RuntimeGlobals::RSPACK_VERSION,
        )));
        Some(true)
      }
      API_UNIQUE_ID => {
        parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
          ident.span.into(),
          RuntimeGlobals::RSPACK_UNIQUE_ID,
        )));
        Some(true)
      }
      API_RSC_MANIFEST => {
        parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
          ident.span.into(),
          RuntimeGlobals::RSC_MANIFEST,
        )));
        Some(true)
      }
      _ => None,
    }
  }

  fn evaluate_identifier(
    &self,
    parser: &mut JavascriptParser,
    for_name: &str,
    start: u32,
    end: u32,
  ) -> Option<eval::BasicEvaluatedExpression<'static>> {
    if for_name == API_LAYER {
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
    if parser
      .compiler_options
      .experiments
      .runtime_mode
      .is_runtime_requirements_proxy_enabled()
      && for_name == API_REQUIRE
      && matches!(member_expr.prop, MemberProp::Computed(_))
    {
      parser
        .add_presentational_dependency(Box::new(RuntimeRequirementsDependency::require_bridge()));
    }

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

  fn member_chain(
    &self,
    parser: &mut JavascriptParser,
    member_expr: &MemberExpr,
    for_name: &str,
    members: &[Atom],
    _members_optionals: &[bool],
    _member_ranges: &[Span],
  ) -> Option<bool> {
    if parser
      .compiler_options
      .experiments
      .runtime_mode
      .is_runtime_requirements_proxy_enabled()
      && (for_name == API_REQUIRE || is_runtime_proxy_name(parser, for_name))
      && let [member] = members
      && let Some(runtime_global) = runtime_globals_from_property_name(member.as_ref())
    {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::new(
        member_expr.span().into(),
        runtime_global,
      )));
      return Some(true);
    }
    None
  }

  fn unhandled_expression_member_chain(
    &self,
    parser: &mut JavascriptParser,
    root_info: &ExportedVariableInfo,
    _expr: &MemberExpr,
  ) -> Option<bool> {
    if parser
      .compiler_options
      .experiments
      .runtime_mode
      .is_runtime_requirements_proxy_enabled()
      && matches!(root_info, ExportedVariableInfo::Name(root) if root == API_REQUIRE)
    {
      parser
        .add_presentational_dependency(Box::new(RuntimeRequirementsDependency::require_bridge()));
    }
    None
  }

  fn assign(
    &self,
    parser: &mut JavascriptParser,
    expr: &swc_core::ecma::ast::AssignExpr,
    for_name: &str,
  ) -> Option<bool> {
    if parser
      .compiler_options
      .experiments
      .runtime_mode
      .is_runtime_requirements_proxy_enabled()
      && let Some(runtime_global) = api_runtime_global(for_name)
    {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::write(
        expr.left.span().into(),
        runtime_global,
      )));
    }
    None
  }

  fn assign_member_chain(
    &self,
    parser: &mut JavascriptParser,
    expr: &swc_core::ecma::ast::AssignExpr,
    members: &[swc_core::atoms::Atom],
    for_name: &str,
  ) -> Option<bool> {
    if parser
      .compiler_options
      .experiments
      .runtime_mode
      .is_runtime_requirements_proxy_enabled()
      && (for_name == API_REQUIRE || is_runtime_proxy_name(parser, for_name))
      && let [member] = members
      && let Some(runtime_global) = runtime_globals_from_property_name(member.as_ref())
    {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::write(
        expr.left.span().into(),
        runtime_global,
      )));
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
