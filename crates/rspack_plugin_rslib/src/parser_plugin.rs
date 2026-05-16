use rspack_plugin_javascript::{
  JavascriptParserPlugin, JavascriptParserPluginHook, visitors::JavascriptParser,
};
use swc_core::ecma::ast::MemberExpr;

#[derive(PartialEq, Debug, Default)]
pub struct RslibParserPlugin {
  intercept_api_plugin: bool,
}

impl RslibParserPlugin {
  pub fn new(intercept_api_plugin: bool) -> Self {
    Self {
      intercept_api_plugin,
    }
  }
}

const RSLIB_MEMBER_NAMES: &[&str] = &[
  "require.cache",
  "require.extensions",
  "require.config",
  "require.version",
  "require.include",
  "require.onError",
];
const RSLIB_TYPEOF_NAMES: &[&str] = &["module"];

#[rspack_plugin_javascript::implemented_javascript_parser_hooks]
impl JavascriptParserPlugin for RslibParserPlugin {
  fn hook_name_filter(&self, hook: JavascriptParserPluginHook) -> Option<&'static [&'static str]> {
    match hook {
      JavascriptParserPluginHook::Member => Some(RSLIB_MEMBER_NAMES),
      JavascriptParserPluginHook::Typeof => Some(RSLIB_TYPEOF_NAMES),
      _ => None,
    }
  }

  fn member(
    &self,
    _parser: &mut JavascriptParser,
    _member_expr: &MemberExpr,
    for_name: &str,
  ) -> Option<bool> {
    if for_name == "require.cache"
      || for_name == "require.extensions"
      || for_name == "require.config"
      || for_name == "require.version"
      || for_name == "require.include"
      || for_name == "require.onError"
    {
      return Some(true);
    }
    None
  }

  fn r#typeof(
    &self,
    _parser: &mut JavascriptParser,
    _expr: &swc_core::ecma::ast::UnaryExpr,
    for_name: &str,
  ) -> Option<bool> {
    if for_name == "module" {
      Some(false)
    } else {
      None
    }
  }
}
