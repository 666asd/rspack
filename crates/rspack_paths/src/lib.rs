use std::{
  collections::{HashMap, HashSet},
  fmt::Debug,
  hash::{BuildHasherDefault, Hash},
  ops::Deref,
  path::{Path, PathBuf},
};

pub use camino::{Utf8Component, Utf8Components, Utf8Path, Utf8PathBuf, Utf8Prefix};
use dashmap::{DashMap, DashSet};
use hstr::Atom;
use indexmap::IndexSet;
use rspack_cacheable::{
  ContextGuard, Error as CacheableError, cacheable,
  utils::PortablePath,
  with::{Custom, CustomConverter},
};
use ustr::IdentityHasher;

pub trait AssertUtf8 {
  type Output;
  fn assert_utf8(self) -> Self::Output;
}

impl AssertUtf8 for PathBuf {
  type Output = Utf8PathBuf;

  /// Assert `self` is a valid UTF-8 [`PathBuf`] and convert to [`Utf8PathBuf`]
  ///
  /// # Panics
  ///
  /// Panics if `self` is not a valid UTF-8 path.
  fn assert_utf8(self) -> Self::Output {
    Utf8PathBuf::from_path_buf(self).unwrap_or_else(|p| {
      panic!("expected UTF-8 path, got: {}", p.display());
    })
  }
}

impl<'a> AssertUtf8 for &'a Path {
  type Output = &'a Utf8Path;

  /// Assert `self` is a valid UTF-8 [`Path`] and convert to [`Utf8Path`]
  ///
  /// # Panics
  ///
  /// Panics if `self` is not a valid UTF-8 path.
  fn assert_utf8(self) -> Self::Output {
    Utf8Path::from_path(self).unwrap_or_else(|| {
      panic!("expected UTF-8 path, got: {}", self.display());
    })
  }
}

#[cacheable(with=Custom)]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ArcPath {
  inner: Atom,
}

impl Debug for ArcPath {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    self.as_ref().fmt(f)
  }
}

impl ArcPath {
  fn from_path(path: &Path) -> Self {
    let path = path.to_str().unwrap_or_else(|| {
      panic!("expected UTF-8 path, got: {}", path.display());
    });
    Self {
      inner: Atom::from(path),
    }
  }
}

impl Deref for ArcPath {
  type Target = Path;

  fn deref(&self) -> &Self::Target {
    self.as_ref()
  }
}

impl AsRef<Path> for ArcPath {
  fn as_ref(&self) -> &Path {
    Path::new(self.inner.as_ref())
  }
}

impl From<PathBuf> for ArcPath {
  fn from(value: PathBuf) -> Self {
    ArcPath::from_path(&value)
  }
}

impl From<&Path> for ArcPath {
  fn from(value: &Path) -> Self {
    ArcPath::from_path(value)
  }
}

impl From<&Utf8Path> for ArcPath {
  fn from(value: &Utf8Path) -> Self {
    ArcPath::from_path(value.as_std_path())
  }
}

impl From<&ArcPath> for ArcPath {
  fn from(value: &ArcPath) -> Self {
    value.clone()
  }
}

impl From<&str> for ArcPath {
  fn from(value: &str) -> Self {
    Self {
      inner: Atom::from(value),
    }
  }
}

impl CustomConverter for ArcPath {
  type Target = PortablePath;
  fn serialize(&self, guard: &ContextGuard) -> Result<Self::Target, CacheableError> {
    Ok(PortablePath::new(self.as_ref(), guard.project_root()))
  }
  fn deserialize(data: Self::Target, guard: &ContextGuard) -> Result<Self, CacheableError> {
    Ok(Self::from(PathBuf::from(
      data.into_path_string(guard.project_root()),
    )))
  }
}

/// A standard `HashMap` using `ArcPath` as the key type with a custom `Hasher`
/// that just uses the `hstr` hash for speed instead of calculating it.
pub type ArcPathMap<V> = HashMap<ArcPath, V, BuildHasherDefault<IdentityHasher>>;

/// A standard `HashSet` using `ArcPath` as the key type with a custom `Hasher`
/// that just uses the `hstr` hash for speed instead of calculating it.
pub type ArcPathSet = HashSet<ArcPath, BuildHasherDefault<IdentityHasher>>;

/// A standard `DashMap` using `ArcPath` as the key type with a custom `Hasher`
/// that just uses the `hstr` hash for speed instead of calculating it.
pub type ArcPathDashMap<V> = DashMap<ArcPath, V, BuildHasherDefault<IdentityHasher>>;

/// A standard `DashSet` using `ArcPath` as the key type with a custom `Hasher`
/// that just uses the `hstr` hash for speed instead of calculating it.
pub type ArcPathDashSet = DashSet<ArcPath, BuildHasherDefault<IdentityHasher>>;

/// A standard `IndexSet` using `ArcPath` as the key type with a custom `Hasher`
/// that just uses the `hstr` hash for speed instead of calculating it.
pub type ArcPathIndexSet = IndexSet<ArcPath, BuildHasherDefault<IdentityHasher>>;
