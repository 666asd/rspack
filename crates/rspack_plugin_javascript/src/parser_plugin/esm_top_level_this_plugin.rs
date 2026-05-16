use rspack_core::ConstDependency;

use super::{JavascriptParserPlugin, JavascriptParserPluginHook};
use crate::visitors::JavascriptParser;

pub struct ESMTopLevelThisParserPlugin;

const THIS_NAMES: &[&str] = &["this"];

#[rspack_macros::implemented_javascript_parser_hooks]
impl JavascriptParserPlugin for ESMTopLevelThisParserPlugin {
  fn hook_name_filter(&self, hook: JavascriptParserPluginHook) -> Option<&'static [&'static str]> {
    match hook {
      JavascriptParserPluginHook::This => Some(THIS_NAMES),
      _ => None,
    }
  }

  fn this(
    &self,
    parser: &mut JavascriptParser,
    expr: &swc_core::ecma::ast::ThisExpr,
    _for_name: &str,
  ) -> Option<bool> {
    (parser.is_esm && parser.is_top_level_this()).then(|| {
      parser.add_presentational_dependency(Box::new(ConstDependency::new(
        expr.span.into(),
        "undefined".into(),
      )));
      true
    })
  }
}
