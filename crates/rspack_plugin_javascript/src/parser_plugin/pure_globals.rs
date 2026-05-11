//! Known-pure global constructors and functions for tree-shaking.
//!
//! When tree-shaking analyses side effects, a `new Set()` or `Boolean(x)` with
//! the real (unresolved) global callee can be treated as side-effect-free if the
//! callee-specific argument rule proves that evaluating the call cannot throw or
//! invoke user code.
//!
//! ## Safety invariants
//!
//! * **Shadowing**: the callee identifier must have the unresolved syntax context
//!   (`ctxt == unresolved_ctxt`), so a module-local `const Set = …` is never
//!   mistaken for the built-in.
//! * **Arguments**: each callee carries a small argument rule. This keeps
//!   coercive or iterable built-ins conservative without making DCE depend on a
//!   full value analysis.

use swc_core::{
  common::SyntaxContext,
  ecma::ast::{Expr, MemberProp},
};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Where the callee appears syntactically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalleePosition {
  /// `new Callee(…)`
  New,
  /// `Callee(…)` or `Callee.method(…)`
  Call,
}

/// The argument rule required before a known global callee can be treated as
/// side-effect-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PureGlobalArgs {
  /// Every argument only needs to be side-effect-free.
  AnyPure,
  /// No arguments, or a single `null`/`undefined` argument.
  NullishOrEmpty,
  /// `Array(...)` / `new Array(...)` constructor rules.
  ArrayConstructor,
  /// Numeric-length constructors such as `ArrayBuffer` and TypedArrays.
  FiniteNonNegativeLength,
  /// `String(...)` or `new String(...)` coercion.
  StringCoercion,
  /// `Number(...)` or `new Number(...)` coercion.
  NumberCoercion,
  /// `new Date(...)`.
  DateConstructor,
  /// `Symbol(...)` description coercion.
  SymbolDescription,
}

/// Classify `callee` as a known-pure global.
///
/// Returns an argument rule when:
/// 1. The callee resolves to an unresolved global (not a local binding).
/// 2. The name + `position` combination is in the allowlist.
pub fn classify_pure_global(
  callee: &Expr,
  unresolved_ctxt: SyntaxContext,
  position: CalleePosition,
) -> Option<PureGlobalArgs> {
  match callee {
    Expr::Ident(ident) if ident.ctxt == unresolved_ctxt => {
      classify_ident(ident.sym.as_str(), position)
    }
    // `Array.isArray(…)` — only valid as a Call.
    Expr::Member(member) if position == CalleePosition::Call => {
      let MemberProp::Ident(prop) = &member.prop else {
        return None;
      };
      let Expr::Ident(obj) = member.obj.as_ref() else {
        return None;
      };
      if obj.ctxt != unresolved_ctxt {
        return None;
      }
      classify_member(obj.sym.as_str(), prop.sym.as_str())
    }
    _ => None,
  }
}

// ---------------------------------------------------------------------------
// Internal classification tables
// ---------------------------------------------------------------------------

fn classify_ident(name: &str, position: CalleePosition) -> Option<PureGlobalArgs> {
  match position {
    CalleePosition::New => classify_new_ident(name),
    CalleePosition::Call => classify_call_ident(name),
  }
}

/// `new Name(…)`
fn classify_new_ident(name: &str) -> Option<PureGlobalArgs> {
  Some(match name {
    "Boolean" | "Object" => PureGlobalArgs::AnyPure,
    "Array" => PureGlobalArgs::ArrayConstructor,
    "Map" | "Set" | "WeakMap" | "WeakSet" => PureGlobalArgs::NullishOrEmpty,
    "ArrayBuffer" | "SharedArrayBuffer" | "BigInt64Array" | "BigUint64Array" | "Float32Array"
    | "Float64Array" | "Int8Array" | "Int16Array" | "Int32Array" | "Uint8Array"
    | "Uint8ClampedArray" | "Uint16Array" | "Uint32Array" => {
      PureGlobalArgs::FiniteNonNegativeLength
    }
    "Date" => PureGlobalArgs::DateConstructor,
    "Number" => PureGlobalArgs::NumberCoercion,
    "String" => PureGlobalArgs::StringCoercion,
    _ => return None,
  })
}

/// `Name(…)`
fn classify_call_ident(name: &str) -> Option<PureGlobalArgs> {
  Some(match name {
    "Boolean" | "Date" | "Object" => PureGlobalArgs::AnyPure,
    "Array" => PureGlobalArgs::ArrayConstructor,
    "Number" => PureGlobalArgs::NumberCoercion,
    "String" => PureGlobalArgs::StringCoercion,
    "Symbol" => PureGlobalArgs::SymbolDescription,
    _ => return None,
  })
}

/// `Obj.method(…)`
///
/// Only methods that are pure type/identity checks and never coerce, iterate,
/// or read properties.
///
/// Notably excluded:
/// * `Object.assign`/`freeze`/`create`/`fromEntries` — mutate or invoke user
///   code via getters/iterators.
/// * `Object.keys`/`values`/`entries` — Proxy `ownKeys`/`get` traps.
/// * `Array.from` — invokes iterator protocol and optional mapper fn.
fn classify_member(obj: &str, prop: &str) -> Option<PureGlobalArgs> {
  let is_known = match obj {
    "Array" => matches!(prop, "isArray" | "of"),
    "Object" => prop == "is",
    "Number" => matches!(prop, "isInteger" | "isFinite" | "isNaN" | "isSafeInteger"),
    _ => false,
  };
  if is_known {
    Some(PureGlobalArgs::AnyPure)
  } else {
    None
  }
}
