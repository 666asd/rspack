use napi_derive::napi;
use rspack_core::Compilation;

#[napi]
pub struct JsDependencies {
  pub(crate) compilation: &'static Compilation,
}

impl JsDependencies {
  pub(crate) fn new(compilation: &'static Compilation) -> Self {
    Self { compilation }
  }
}

#[napi]
impl JsDependencies {
  #[napi(getter)]
  pub fn file_dependencies(&self) -> Vec<String> {
    self
      .compilation
      .file_dependencies()
      .0
      .map(|i| i.to_string_lossy().to_string())
      .collect()
  }
  // `added_*_dependencies` merges `added` with `updated` so the JS-side
  // native watcher protocol (incremental `(added, removed)` deltas) re-sees
  // paths that went through a remove+add cycle in this build — for example,
  // a lazy-compiled module whose owning dependency was revoked and
  // re-factorized. PathTracker.add is idempotent on the native side, so
  // re-adding tracked paths is safe; without this, FSEvents on those paths
  // would be silently dropped by DependencyFinder. See #12904.
  #[napi(getter)]
  pub fn added_file_dependencies(&self) -> Vec<String> {
    let (_, added, updated, _) = self.compilation.file_dependencies();
    added
      .chain(updated)
      .map(|i| i.to_string_lossy().to_string())
      .collect()
  }
  #[napi(getter)]
  pub fn removed_file_dependencies(&self) -> Vec<String> {
    self
      .compilation
      .file_dependencies()
      .3
      .map(|i| i.to_string_lossy().to_string())
      .collect()
  }

  #[napi(getter)]
  pub fn context_dependencies(&self) -> Vec<String> {
    self
      .compilation
      .context_dependencies()
      .0
      .map(|i| i.to_string_lossy().to_string())
      .collect()
  }
  #[napi(getter)]
  pub fn added_context_dependencies(&self) -> Vec<String> {
    let (_, added, updated, _) = self.compilation.context_dependencies();
    added
      .chain(updated)
      .map(|i| i.to_string_lossy().to_string())
      .collect()
  }
  #[napi(getter)]
  pub fn removed_context_dependencies(&self) -> Vec<String> {
    self
      .compilation
      .context_dependencies()
      .3
      .map(|i| i.to_string_lossy().to_string())
      .collect()
  }

  #[napi(getter)]
  pub fn missing_dependencies(&self) -> Vec<String> {
    self
      .compilation
      .missing_dependencies()
      .0
      .map(|i| i.to_string_lossy().to_string())
      .collect()
  }
  #[napi(getter)]
  pub fn added_missing_dependencies(&self) -> Vec<String> {
    let (_, added, updated, _) = self.compilation.missing_dependencies();
    added
      .chain(updated)
      .map(|i| i.to_string_lossy().to_string())
      .collect()
  }
  #[napi(getter)]
  pub fn removed_missing_dependencies(&self) -> Vec<String> {
    self
      .compilation
      .missing_dependencies()
      .3
      .map(|i| i.to_string_lossy().to_string())
      .collect()
  }

  #[napi(getter)]
  pub fn build_dependencies(&self) -> Vec<String> {
    self
      .compilation
      .build_dependencies()
      .0
      .map(|i| i.to_string_lossy().to_string())
      .collect()
  }
  #[napi(getter)]
  pub fn added_build_dependencies(&self) -> Vec<String> {
    let (_, added, updated, _) = self.compilation.build_dependencies();
    added
      .chain(updated)
      .map(|i| i.to_string_lossy().to_string())
      .collect()
  }
  #[napi(getter)]
  pub fn removed_build_dependencies(&self) -> Vec<String> {
    self
      .compilation
      .build_dependencies()
      .3
      .map(|i| i.to_string_lossy().to_string())
      .collect()
  }
}
