use rspack_core::{ConstDependency, RuntimeGlobals, RuntimeRequirementsDependency};
use swc_core::ecma::ast::MemberExpr;

use super::{JavascriptParserPlugin, JavascriptParserPluginHook};
use crate::{
  utils::eval::{BasicEvaluatedExpression, evaluate_to_identifier},
  visitors::{JavascriptParser, expr_name},
};

pub struct CommonJsPlugin;

const COMMON_JS_EVALUATE_IDENTIFIER_NAMES: &[&str] = &[expr_name::MODULE_HOT];
const COMMON_JS_TYPEOF_NAMES: &[&str] = &[expr_name::MODULE];
const COMMON_JS_MEMBER_NAMES: &[&str] = &["module.id", "module.loaded"];

#[rspack_macros::implemented_javascript_parser_hooks]
impl JavascriptParserPlugin for CommonJsPlugin {
  fn hook_name_filter(&self, hook: JavascriptParserPluginHook) -> Option<&'static [&'static str]> {
    match hook {
      JavascriptParserPluginHook::EvaluateIdentifier => Some(COMMON_JS_EVALUATE_IDENTIFIER_NAMES),
      JavascriptParserPluginHook::Typeof => Some(COMMON_JS_TYPEOF_NAMES),
      JavascriptParserPluginHook::Member => Some(COMMON_JS_MEMBER_NAMES),
      _ => None,
    }
  }

  fn evaluate_identifier(
    &self,
    _parser: &mut JavascriptParser,
    for_name: &str,
    start: u32,
    end: u32,
  ) -> Option<BasicEvaluatedExpression<'static>> {
    if for_name == expr_name::MODULE_HOT {
      Some(evaluate_to_identifier(
        expr_name::MODULE_HOT.into(),
        expr_name::MODULE.into(),
        None,
        start,
        end,
      ))
    } else {
      None
    }
  }

  fn r#typeof(
    &self,
    parser: &mut JavascriptParser,
    expr: &swc_core::ecma::ast::UnaryExpr,
    for_name: &str,
  ) -> Option<bool> {
    if for_name == expr_name::MODULE {
      parser.add_presentational_dependency(Box::new(ConstDependency::new(
        expr.span.into(),
        "'object'".into(),
      )));
      Some(true)
    } else {
      None
    }
  }

  fn member(
    &self,
    parser: &mut JavascriptParser,
    _expr: &MemberExpr,
    for_name: &str,
  ) -> Option<bool> {
    if for_name == "module.id" {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::add_only(
        RuntimeGlobals::MODULE_ID,
      )));
      parser.build_info.module_concatenation_bailout = Some(for_name.to_string());
      return Some(true);
    }

    if for_name == "module.loaded" {
      parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::add_only(
        RuntimeGlobals::MODULE_LOADED,
      )));
      parser.build_info.module_concatenation_bailout = Some(for_name.to_string());
      return Some(true);
    }

    None
  }
}
