use swc_core::{
  atoms::Atom,
  ecma::ast::{Expr, MemberExpr, OptChainExpr},
};

use super::{AllowedMemberTypes, ExportedVariableInfo, JavascriptParser, MemberExpressionInfo};
use crate::visitors::{ExprRef, scope_info::VariableInfoId};

#[derive(Clone, Copy, Debug)]
pub enum ParserHookName<'a> {
  Identifier(&'a Atom),
  MemberChain(&'a str),
}

impl<'a> ParserHookName<'a> {
  pub fn as_str(self) -> &'a str {
    match self {
      Self::Identifier(name) => name,
      Self::MemberChain(name) => name,
    }
  }

  #[inline]
  pub fn is_identifier(self, expected: &Atom) -> bool {
    match self {
      Self::Identifier(name) => name == expected,
      Self::MemberChain(_) => false,
    }
  }

  #[inline]
  pub fn is_member_chain(self, expected: &str) -> bool {
    match self {
      Self::Identifier(_) => false,
      Self::MemberChain(name) => name == expected,
    }
  }
}

/// callHooksForName/callHooksForInfo in webpack
/// webpack use HookMap and filter at callHooksForName/callHooksForInfo
/// we need to pass the name to hook to filter in the hook
pub trait CallHooksName {
  fn call_hooks_name<F, T>(&self, parser: &mut JavascriptParser, hook_call: F) -> Option<T>
  where
    F: for<'name> Fn(&mut JavascriptParser, ParserHookName<'name>) -> Option<T>;
}

#[allow(unused_lifetimes)]
impl CallHooksName for Atom {
  fn call_hooks_name<'parser, F, T>(&self, parser: &mut JavascriptParser, hook_call: F) -> Option<T>
  where
    F: for<'name> Fn(&mut JavascriptParser, ParserHookName<'name>) -> Option<T>,
  {
    if let Some(id) = parser.get_variable_info(self).map(|info| info.id()) {
      // resolved variable info
      call_hooks_info(id, parser, hook_call)
    } else {
      // unresolved free variable, for example the global `require` in commonjs.
      hook_call(parser, ParserHookName::Identifier(self))
    }
  }
}

impl CallHooksName for &str {
  fn call_hooks_name<F, T>(&self, parser: &mut JavascriptParser, hook_call: F) -> Option<T>
  where
    F: for<'name> Fn(&mut JavascriptParser, ParserHookName<'name>) -> Option<T>,
  {
    Atom::from(*self).call_hooks_name(parser, hook_call)
  }
}
#[allow(unused_lifetimes)]
impl CallHooksName for String {
  fn call_hooks_name<'parser, F, T>(&self, parser: &mut JavascriptParser, hook_call: F) -> Option<T>
  where
    F: for<'name> Fn(&mut JavascriptParser, ParserHookName<'name>) -> Option<T>,
  {
    hook_call(parser, ParserHookName::MemberChain(self))
  }
}
#[allow(unused_lifetimes)]
impl CallHooksName for ExportedVariableInfo {
  fn call_hooks_name<'parser, F, T>(
    &self,
    parser: &mut JavascriptParser,
    hooks_call: F,
  ) -> Option<T>
  where
    F: for<'name> Fn(&mut JavascriptParser, ParserHookName<'name>) -> Option<T>,
  {
    match self {
      ExportedVariableInfo::Name(n) => n.call_hooks_name(parser, hooks_call),
      ExportedVariableInfo::VariableInfo(v) => call_hooks_info(*v, parser, hooks_call),
    }
  }
}
#[allow(unused_lifetimes)]
impl CallHooksName for MemberExpr {
  fn call_hooks_name<'parser, F, T>(&self, parser: &mut JavascriptParser, hook_call: F) -> Option<T>
  where
    F: for<'name> Fn(&mut JavascriptParser, ParserHookName<'name>) -> Option<T>,
  {
    let Some(MemberExpressionInfo::Expression(expr_name)) =
      parser.get_member_expression_info(ExprRef::Member(self), AllowedMemberTypes::Expression)
    else {
      return None;
    };

    let members = expr_name.members;
    if members.is_empty() {
      expr_name.root_info.call_hooks_name(parser, hook_call)
    } else {
      expr_name.name.call_hooks_name(parser, hook_call)
    }
  }
}
#[allow(unused_lifetimes)]
impl CallHooksName for OptChainExpr {
  fn call_hooks_name<'parser, F, T>(&self, parser: &mut JavascriptParser, hook_call: F) -> Option<T>
  where
    F: for<'name> Fn(&mut JavascriptParser, ParserHookName<'name>) -> Option<T>,
  {
    let Some(MemberExpressionInfo::Expression(expr_name)) = parser
      .get_member_expression_info_from_expr(
        &Expr::OptChain(self.to_owned()),
        AllowedMemberTypes::Expression,
      )
    else {
      return None;
    };

    let members = expr_name.members;
    if members.is_empty() {
      expr_name.root_info.call_hooks_name(parser, hook_call)
    } else {
      expr_name.name.call_hooks_name(parser, hook_call)
    }
  }
}

fn call_hooks_info<F, T>(
  id: VariableInfoId,
  parser: &mut JavascriptParser,
  hook_call: F,
) -> Option<T>
where
  F: for<'name> Fn(&mut JavascriptParser, ParserHookName<'name>) -> Option<T>,
{
  let info = parser.definitions_db.expect_get_variable(id);
  let mut next_tag_info = info.tag_info;

  while let Some(tag_info_id) = next_tag_info {
    parser.current_tag_info = Some(tag_info_id);
    let (next, tag) = {
      let tag_info = parser.definitions_db.expect_get_tag_info(tag_info_id);
      (tag_info.next, tag_info.tag)
    };
    let result = hook_call(parser, ParserHookName::MemberChain(tag));
    parser.current_tag_info = None;
    if result.is_some() {
      return result;
    }
    next_tag_info = next;
  }

  let name = {
    let info = parser.definitions_db.expect_get_variable(id);
    if info.is_free() || info.is_tagged() {
      info.name.clone()
    } else {
      None
    }
  };
  if let Some(name) = &name {
    let result = hook_call(parser, ParserHookName::Identifier(name));
    if result.is_some() {
      return result;
    }
  }

  None
}
