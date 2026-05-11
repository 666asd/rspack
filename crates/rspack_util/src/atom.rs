use std::{
  collections::{HashMap, HashSet},
  hash::BuildHasherDefault,
};

use indexmap::{IndexMap, IndexSet};
use swc_core::ecma::ast::ModuleExportName;
use ustr::IdentityHasher;

pub type Atom = swc_core::atoms::Atom;

pub type AtomHashSet = HashSet<Atom, BuildHasherDefault<IdentityHasher>>;
pub type AtomHashMap<V> = HashMap<Atom, V, BuildHasherDefault<IdentityHasher>>;
pub type AtomIndexSet = IndexSet<Atom, BuildHasherDefault<IdentityHasher>>;
pub type AtomIndexMap<V> = IndexMap<Atom, V, BuildHasherDefault<IdentityHasher>>;

pub trait ModuleExportNameExt {
  fn atom_ref(&self) -> &Atom;
}

impl ModuleExportNameExt for ModuleExportName {
  fn atom_ref(&self) -> &Atom {
    match self {
      ModuleExportName::Ident(ident) => &ident.sym,
      ModuleExportName::Str(s) => s
        .value
        .as_atom()
        .expect("ModuleExportName should be a valid utf8"),
    }
  }
}
