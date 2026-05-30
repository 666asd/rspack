use rspack_core::ConstDependency;
use rspack_util::SpanExt;
use swc_core::{
  atoms::Atom,
  common::Spanned,
  ecma::ast::{CallExpr, UnaryExpr},
};

use super::JavascriptParserPlugin;
use crate::{
  dependency::IsIncludeDependency,
  visitors::{JavascriptParser, ParserHookName},
};

const IS_INCLUDED: &str = "__webpack_is_included__";

thread_local! {
  static IS_INCLUDED_ATOM: Atom = Atom::from(IS_INCLUDED);
}

#[inline]
fn is_included_identifier(for_name: ParserHookName<'_>) -> bool {
  if !matches!(for_name.as_atom(), Some(name) if name.len() == IS_INCLUDED.len()) {
    return false;
  }
  IS_INCLUDED_ATOM.with(|atom| for_name.is_identifier(atom))
}

pub struct IsIncludedPlugin;

#[rspack_macros::implemented_javascript_parser_hooks]
impl JavascriptParserPlugin for IsIncludedPlugin {
  fn call(&self, parser: &mut JavascriptParser, expr: &CallExpr, name: &str) -> Option<bool> {
    if name != IS_INCLUDED || expr.args.len() != 1 || expr.args[0].spread.is_some() {
      return None;
    }

    let request = parser.evaluate_expression(&expr.args[0].expr);
    if !request.is_string() {
      return None;
    }

    parser.add_dependency(Box::new(IsIncludeDependency::new(
      (expr.span().real_lo(), expr.span().real_hi()).into(),
      request.string().clone(),
    )));

    Some(true)
  }

  fn r#typeof(
    &self,
    parser: &mut JavascriptParser<'_>,
    expr: &UnaryExpr,
    for_name: ParserHookName<'_>,
  ) -> Option<bool> {
    is_included_identifier(for_name).then(|| {
      parser.add_presentational_dependency(Box::new(ConstDependency::new(
        (expr.span().real_lo(), expr.span().real_hi()).into(),
        "'function'".into(),
      )));
      true
    })
  }
}
