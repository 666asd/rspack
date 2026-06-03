use std::cell::{Cell, UnsafeCell};

use bumpalo::{Bump, collections::Vec as BumpVec};

/// A scoped bump allocator for temporary source-map scratch storage.
///
/// `ObjectPool` is reused through thread-local storage by devtool/minimizer code.
/// Each top-level source-map generation enters an allocation scope, allocates
/// UTF-16 byte-index scratch vectors from the bump arena, and resets the arena
/// when the outermost scope exits. Direct unscoped `pull` calls fall back to an
/// owned `Vec` so public helper usage stays safe even without a scope.
#[derive(Debug)]
pub struct ObjectPool {
  bump: UnsafeCell<Bump>,
  active_scopes: Cell<usize>,
}

impl Default for ObjectPool {
  fn default() -> Self {
    Self {
      bump: UnsafeCell::new(Bump::new()),
      active_scopes: Cell::new(0),
    }
  }
}

impl ObjectPool {
  /// Enter a temporary allocation scope.
  ///
  /// Nested scopes share the same arena. The arena is reset only after the
  /// outermost scope exits, so temporary allocations remain valid for the whole
  /// source-map streaming call that created them.
  pub(crate) fn scope(&self) -> Scope<'_> {
    let depth = self.active_scopes.get();
    if depth == 0 {
      self.reset();
    }
    self.active_scopes.set(depth + 1);
    Scope { pool: self }
  }

  /// Retrieves temporary `usize` scratch storage with at least the requested capacity.
  pub fn pull<'a>(&'a self, requested_capacity: usize) -> Pooled<'a> {
    if self.active_scopes.get() == 0 {
      return Pooled::new_heap(requested_capacity);
    }

    Pooled::new_in(requested_capacity, self.bump())
  }

  #[inline]
  fn bump(&self) -> &Bump {
    #[allow(unsafe_code)]
    // SAFETY: `Bump` allocation only requires shared access. `ObjectPool` is
    // intentionally `!Sync` because it contains `Cell`/`UnsafeCell`, so shared
    // references are not used concurrently across threads.
    unsafe {
      &*self.bump.get()
    }
  }

  #[inline]
  fn reset(&self) {
    #[allow(unsafe_code)]
    // SAFETY: `reset` is called only when `active_scopes == 0`, before a new
    // scope starts or after the outermost scope exits. All bump-backed
    // `Pooled` values created by internal streaming code have been dropped at
    // that point.
    unsafe {
      (&mut *self.bump.get()).reset();
    }
  }
}

/// Guard for an [`ObjectPool`] temporary allocation scope.
pub(crate) struct Scope<'pool> {
  pool: &'pool ObjectPool,
}

impl Drop for Scope<'_> {
  fn drop(&mut self) {
    let depth = self.pool.active_scopes.get();
    debug_assert!(depth > 0);
    let next_depth = depth.saturating_sub(1);
    self.pool.active_scopes.set(next_depth);
    if next_depth == 0 {
      self.pool.reset();
    }
  }
}

/// Temporary `usize` scratch storage.
#[derive(Debug)]
pub struct Pooled<'object_pool> {
  inner: PooledInner<'object_pool>,
}

#[derive(Debug)]
enum PooledInner<'object_pool> {
  Bump(BumpVec<'object_pool, usize>),
  Heap(Vec<usize>),
}

impl<'object_pool> Pooled<'object_pool> {
  fn new_in(capacity: usize, bump: &'object_pool Bump) -> Self {
    Self {
      inner: PooledInner::Bump(BumpVec::with_capacity_in(capacity, bump)),
    }
  }

  fn new_heap(capacity: usize) -> Self {
    Self {
      inner: PooledInner::Heap(Vec::with_capacity(capacity)),
    }
  }

  pub fn as_mut(&mut self) -> &mut [usize] {
    match &mut self.inner {
      PooledInner::Bump(vec) => vec.as_mut_slice(),
      PooledInner::Heap(vec) => vec.as_mut_slice(),
    }
  }

  pub fn as_ref(&self) -> &[usize] {
    match &self.inner {
      PooledInner::Bump(vec) => vec.as_slice(),
      PooledInner::Heap(vec) => vec.as_slice(),
    }
  }

  pub fn push(&mut self, value: usize) {
    match &mut self.inner {
      PooledInner::Bump(vec) => vec.push(value),
      PooledInner::Heap(vec) => vec.push(value),
    }
  }
}

impl std::ops::Deref for Pooled<'_> {
  type Target = [usize];

  fn deref(&self) -> &Self::Target {
    self.as_ref()
  }
}

impl std::ops::DerefMut for Pooled<'_> {
  fn deref_mut(&mut self) -> &mut Self::Target {
    self.as_mut()
  }
}
