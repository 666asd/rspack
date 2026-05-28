//! SWC plugin runtime registration.
//!
//! Rspack's Node.js binding registers a runtime backed by Node/V8's built-in
//! `WebAssembly` implementation at module initialization time. Keeping the
//! runtime behind a process-wide registration point lets non-Node crates avoid a
//! direct N-API dependency while still allowing `builtin:swc-loader` and the SWC
//! transform API to use the same runtime.

use std::{
  fmt,
  path::Path,
  sync::{Arc, OnceLock, RwLock},
};

use swc_plugin_runner::runtime;

static PLUGIN_RUNTIME: OnceLock<RwLock<Option<Arc<dyn runtime::Runtime>>>> = OnceLock::new();

fn runtime_slot() -> &'static RwLock<Option<Arc<dyn runtime::Runtime>>> {
  PLUGIN_RUNTIME.get_or_init(Default::default)
}

/// Registers the process-wide SWC Wasm plugin runtime.
///
/// The Node.js binding calls this with a runtime backed by Node's built-in
/// `WebAssembly` implementation. Calling it again replaces the previous runtime,
/// which is useful for tests and for reloading the binding in the same process.
pub fn set_plugin_runtime(runtime: Arc<dyn runtime::Runtime>) {
  *runtime_slot()
    .write()
    .expect("SWC plugin runtime slot should not be poisoned") = Some(runtime);
}

/// Returns the registered SWC Wasm plugin runtime.
///
/// If no runtime has been registered yet, a placeholder runtime is returned. The
/// placeholder reports a clear error when a Wasm plugin is actually used instead
/// of failing at configuration time for builds that do not use Wasm plugins.
pub fn plugin_runtime() -> Arc<dyn runtime::Runtime> {
  runtime_slot()
    .read()
    .expect("SWC plugin runtime slot should not be poisoned")
    .clone()
    .unwrap_or_else(|| Arc::new(MissingPluginRuntime))
}

#[derive(Clone, Copy)]
struct MissingPluginRuntime;

impl fmt::Debug for MissingPluginRuntime {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("MissingPluginRuntime").finish()
  }
}

impl runtime::Runtime for MissingPluginRuntime {
  fn identifier(&self) -> &'static str {
    "missing-node-wasm-runtime"
  }

  fn prepare_module(&self, _bytes: &[u8]) -> anyhow::Result<runtime::ModuleCache> {
    anyhow::bail!(missing_runtime_message())
  }

  fn init(
    &self,
    _name: &str,
    _imports: Vec<(String, runtime::Func)>,
    _envs: Vec<(String, String)>,
    _module: runtime::Module,
  ) -> anyhow::Result<Box<dyn runtime::Instance>> {
    anyhow::bail!(missing_runtime_message())
  }

  fn clone_cache(&self, _cache: &runtime::ModuleCache) -> Option<runtime::ModuleCache> {
    None
  }

  unsafe fn load_cache(&self, _path: &Path) -> Option<runtime::ModuleCache> {
    None
  }
}

fn missing_runtime_message() -> &'static str {
  "SWC Wasm plugin runtime is not registered. Rspack's Node.js binding registers the runtime backed by Node.js built-in WebAssembly during module initialization; make sure @rspack/binding is loaded through its JavaScript entry."
}
