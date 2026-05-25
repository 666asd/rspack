use std::sync::LazyLock;

use bitflags::bitflags;
use rustc_hash::FxHashMap;

use crate::CompilerOptions;

#[rspack_cacheable::cacheable]
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct RuntimeGlobals(u128);

macro_rules! define_runtime_globals {
  (@step ($val:expr) ({$($acc:tt)*}) ($(#[$($attr:tt)+])* const $name:ident; $($rest:tt)*)) => {
    define_runtime_globals! {
      @step ($val << 1) ({
        $($acc)*
        $(#[$($attr)+])*
        const $name = $val;
      }) ($($rest)*)
    }
  };
  (@step ($val:expr) ({$($acc:tt)*}) ()) => {
    bitflags! {
      impl RuntimeGlobals: u128 {
        $($acc)*
      }
    }
  };
  ($($rest:tt)*) => {
    define_runtime_globals! {
      @step (1u128) ({}) ($($rest)*)
    }
  };
}

define_runtime_globals! {
  const REQUIRE_SCOPE;

  /**
   * the internal module object
   */
  const MODULE;

  /**
   * the internal module object
   */
  const MODULE_ID;

  /**
   * the internal require function
   */
  const REQUIRE;

  /**
   * the module cache
   */
  const MODULE_CACHE;

  /**
   * the chunk ensure function
   */
  const ENSURE_CHUNK;

  /**
   * an object with handlers to ensure a chunk
   */
  const ENSURE_CHUNK_HANDLERS;

  /**
   * the bundle public path
   */
  const PUBLIC_PATH;

  /**
   * the filename of the script part of the chunk
   */
  const GET_CHUNK_SCRIPT_FILENAME;

  /**
   * the filename of the css part of the chunk
   */
  const GET_CHUNK_CSS_FILENAME;

  /**
   * function to load a script tag.
   * Arguments: (url: string, done: (event) => void), key?: string | number, chunkId?: string | number) => void
   * done function is called when loading has finished or timeout occurred.
   * It will attach to existing script tags with data-webpack == uniqueName + ":" + key or src == url.
   */
  const LOAD_SCRIPT;

  /**
   * the shorthand for Object.prototype.hasOwnProperty
   * using of it decreases the compiled bundle size
   */
  const HAS_OWN_PROPERTY;

  /**
   * the module functions, with only write access
   */
  const MODULE_FACTORIES_ADD_ONLY;

  /**
   * register deferred code, which will run when certain
   * chunks are loaded.
   * Signature: (chunkIds: Id[], fn: () => any, priority: int >= 0 = 0) => any
   * Returned value will be returned directly when all chunks are already loaded
   * When (priority & 1) it will wait for all other handlers with lower priority to
   * be executed before itself is executed
   */
  const ON_CHUNKS_LOADED;

  /**
   * global callback functions for installing chunks
   */
  const CHUNK_CALLBACK;

  /**
   * the module functions
   */
  const MODULE_FACTORIES;

  /**
   * interceptor for module executions
   */
  const INTERCEPT_MODULE_EXECUTION;

  /**
   * function downloading the update manifest
   */
  const HMR_DOWNLOAD_MANIFEST;

  /**
   * array with handler functions to download chunk updates
   */
  const HMR_DOWNLOAD_UPDATE_HANDLERS;

  const HMR_INVALIDATE_MODULE_HANDLERS;

  /**
   * the filename of the HMR manifest
   */
  const GET_UPDATE_MANIFEST_FILENAME;

  /**
   * the filename of the script part of the hot update chunk
   */
  const GET_CHUNK_UPDATE_SCRIPT_FILENAME;

  /**
   * the filename of the css part of the hot update chunk
   */
  const GET_CHUNK_UPDATE_CSS_FILENAME;

  /**
   * object with all hmr module data for all modules
   */
  const HMR_MODULE_DATA;

  /**
   * the prefix for storing state of runtime modules when hmr is enabled
   */
  const HMR_RUNTIME_STATE_PREFIX;

  /**
   * method to install a chunk that was loaded somehow
   * Signature: ({ id, ids, modules, runtime }) => void
   */
  const EXTERNAL_INSTALL_CHUNK;

  /**
   * the webpack hash
   */
  const GET_FULL_HASH;

  /**
   * the global object
   */
  const GLOBAL;

  /**
   * runtime need to return the exports of the last entry module
   */
  const RETURN_EXPORTS_FROM_RUNTIME;

  /**
   * instantiate a wasm instance from module exports object, id, hash and importsObject
   */
  const INSTANTIATE_WASM;

  /**
   * Creates an async module. The body function must be a async function.
   * "module.exports" will be decorated with an AsyncModulePromise.
   * The body function will be called.
   * To handle async dependencies correctly do this: "([a, b, c] = await handleDependencies([a, b, c]));".
   * If "hasAwaitAfterDependencies" is truthy, "handleDependencies()" must be called at the end of the body function.
   * Signature: function(
   * module: Module,
   * body: (handleDependencies: (deps: AsyncModulePromise[]) => Promise<any[]> & () => void,
   * hasAwaitAfterDependencies?: boolean
   * ) => void
   */
  const ASYNC_MODULE;

  /**
   * the baseURI of current document
   */
  const BASE_URI;

  const MODULE_LOADED;

  const STARTUP_ENTRYPOINT;
  const STARTUP_CHUNK_DEPENDENCIES;

  const CREATE_SCRIPT_URL;

  const CREATE_SCRIPT;

  const GET_TRUSTED_TYPES_POLICY;

  const DEFINE_PROPERTY_GETTERS;

  const ENTRY_MODULE_ID;

  const STARTUP_NO_DEFAULT;

  const ENSURE_CHUNK_INCLUDE_ENTRIES;

  const STARTUP;

  const MAKE_NAMESPACE_OBJECT;

  const EXPORTS;

  const COMPAT_GET_DEFAULT_EXPORT;

  const CREATE_FAKE_NAMESPACE_OBJECT;

  const NODE_MODULE_DECORATOR;

  const ESM_MODULE_DECORATOR;

  /**
   * the System.register context object
   */
  const SYSTEM_CONTEXT;

  const THIS_AS_EXPORTS;

  const CURRENT_REMOTE_GET_SCOPE;

  const SHARE_SCOPE_MAP;

  const INITIALIZE_SHARING;

  const SCRIPT_NONCE;

  const RELATIVE_URL;

  const CHUNK_NAME;

  const RUNTIME_ID;

  // prefetch and preload
  const PREFETCH_CHUNK;

  const PREFETCH_CHUNK_HANDLERS;

  const PRELOAD_CHUNK;

  const PRELOAD_CHUNK_HANDLERS;

  const UNCAUGHT_ERROR_HANDLER;

  // rspack only
  const RSPACK_VERSION;

  const HAS_CSS_MODULES;

  // rspack only
  const RSPACK_UNIQUE_ID;

  const HAS_FETCH_PRIORITY;

  // amd module support
  const AMD_DEFINE;
  const AMD_OPTIONS;

  const TO_BINARY;

  // defer import support
  const ASYNC_MODULE_EXPORT_SYMBOL;
  const MAKE_DEFERRED_NAMESPACE_OBJECT;
  const MAKE_OPTIMIZED_DEFERRED_NAMESPACE_OBJECT;
  const DEFERRED_MODULES_ASYNC_TRANSITIVE_DEPENDENCIES;
  const DEFERRED_MODULES_ASYNC_TRANSITIVE_DEPENDENCIES_SYMBOL;

  // rspack only
  const ASYNC_STARTUP;

  // react server component
  const RSC_MANIFEST;
}

impl Default for RuntimeGlobals {
  fn default() -> Self {
    Self::empty()
  }
}

pub static REQUIRE_SCOPE_GLOBALS: LazyLock<RuntimeGlobals> = LazyLock::new(|| {
  RuntimeGlobals::REQUIRE_SCOPE
    | RuntimeGlobals::MODULE_CACHE
    | RuntimeGlobals::ENSURE_CHUNK
    | RuntimeGlobals::ENSURE_CHUNK_HANDLERS
    | RuntimeGlobals::PUBLIC_PATH
    | RuntimeGlobals::GET_CHUNK_SCRIPT_FILENAME
    | RuntimeGlobals::GET_CHUNK_CSS_FILENAME
    | RuntimeGlobals::LOAD_SCRIPT
    | RuntimeGlobals::HAS_OWN_PROPERTY
    | RuntimeGlobals::MODULE_FACTORIES_ADD_ONLY
    | RuntimeGlobals::ON_CHUNKS_LOADED
    | RuntimeGlobals::MODULE_FACTORIES
    | RuntimeGlobals::INTERCEPT_MODULE_EXECUTION
    | RuntimeGlobals::HMR_DOWNLOAD_MANIFEST
    | RuntimeGlobals::HMR_DOWNLOAD_UPDATE_HANDLERS
    | RuntimeGlobals::HMR_INVALIDATE_MODULE_HANDLERS
    | RuntimeGlobals::HMR_MODULE_DATA
    | RuntimeGlobals::HMR_RUNTIME_STATE_PREFIX
    | RuntimeGlobals::GET_UPDATE_MANIFEST_FILENAME
    | RuntimeGlobals::GET_CHUNK_UPDATE_SCRIPT_FILENAME
    | RuntimeGlobals::GET_CHUNK_UPDATE_CSS_FILENAME
    | RuntimeGlobals::AMD_DEFINE
    | RuntimeGlobals::AMD_OPTIONS
    | RuntimeGlobals::EXTERNAL_INSTALL_CHUNK
    | RuntimeGlobals::GET_FULL_HASH
    | RuntimeGlobals::GLOBAL
    | RuntimeGlobals::INSTANTIATE_WASM
    | RuntimeGlobals::ASYNC_MODULE
    | RuntimeGlobals::ASYNC_MODULE_EXPORT_SYMBOL
    | RuntimeGlobals::BASE_URI
    | RuntimeGlobals::STARTUP_ENTRYPOINT
    | RuntimeGlobals::STARTUP_CHUNK_DEPENDENCIES
    | RuntimeGlobals::CREATE_SCRIPT_URL
    | RuntimeGlobals::CREATE_SCRIPT
    | RuntimeGlobals::GET_TRUSTED_TYPES_POLICY
    | RuntimeGlobals::DEFINE_PROPERTY_GETTERS
    | RuntimeGlobals::ENTRY_MODULE_ID
    | RuntimeGlobals::STARTUP_NO_DEFAULT
    | RuntimeGlobals::ENSURE_CHUNK_INCLUDE_ENTRIES
    | RuntimeGlobals::STARTUP
    | RuntimeGlobals::MAKE_NAMESPACE_OBJECT
    | RuntimeGlobals::MAKE_DEFERRED_NAMESPACE_OBJECT
    | RuntimeGlobals::MAKE_OPTIMIZED_DEFERRED_NAMESPACE_OBJECT
    | RuntimeGlobals::COMPAT_GET_DEFAULT_EXPORT
    | RuntimeGlobals::CREATE_FAKE_NAMESPACE_OBJECT
    | RuntimeGlobals::ESM_MODULE_DECORATOR
    | RuntimeGlobals::NODE_MODULE_DECORATOR
    | RuntimeGlobals::SYSTEM_CONTEXT
    | RuntimeGlobals::CURRENT_REMOTE_GET_SCOPE
    | RuntimeGlobals::SHARE_SCOPE_MAP
    | RuntimeGlobals::INITIALIZE_SHARING
    | RuntimeGlobals::SCRIPT_NONCE
    | RuntimeGlobals::RELATIVE_URL
    | RuntimeGlobals::CHUNK_NAME
    | RuntimeGlobals::RUNTIME_ID
    | RuntimeGlobals::PREFETCH_CHUNK
    | RuntimeGlobals::PREFETCH_CHUNK_HANDLERS
    | RuntimeGlobals::PRELOAD_CHUNK
    | RuntimeGlobals::PRELOAD_CHUNK_HANDLERS
    | RuntimeGlobals::UNCAUGHT_ERROR_HANDLER
    | RuntimeGlobals::RSPACK_VERSION
    | RuntimeGlobals::RSPACK_UNIQUE_ID
    | RuntimeGlobals::ASYNC_STARTUP
    | RuntimeGlobals::RSC_MANIFEST
    | RuntimeGlobals::TO_BINARY
    | RuntimeGlobals::DEFERRED_MODULES_ASYNC_TRANSITIVE_DEPENDENCIES
    | RuntimeGlobals::DEFERRED_MODULES_ASYNC_TRANSITIVE_DEPENDENCIES_SYMBOL
});

/// Runtime globals that can be rendered as concrete require-scope properties.
///
/// For example, `RuntimeGlobals::DEFINE_PROPERTY_GETTERS` is included and can
/// render as `__webpack_require__.d`, while `RuntimeGlobals::REQUIRE_SCOPE` is
/// excluded because it represents the broad `__webpack_require__.*` surface.
pub static RENDERABLE_REQUIRE_SCOPE_GLOBALS: LazyLock<RuntimeGlobals> = LazyLock::new(|| {
  let mut runtime_globals = *REQUIRE_SCOPE_GLOBALS;
  runtime_globals.remove(RuntimeGlobals::REQUIRE_SCOPE);
  runtime_globals.remove(RuntimeGlobals::HMR_RUNTIME_STATE_PREFIX);
  runtime_globals
});

pub static MODULE_GLOBALS: LazyLock<RuntimeGlobals> =
  LazyLock::new(|| RuntimeGlobals::MODULE_ID | RuntimeGlobals::MODULE_LOADED);

/// Renders a runtime global in the legacy runtime surface.
///
/// For example, `RuntimeGlobals::DEFINE_PROPERTY_GETTERS` renders to
/// `__webpack_require__.d`, while `RuntimeGlobals::EXPORTS` renders to
/// `__webpack_exports__`.
pub fn runtime_globals_to_string(
  runtime_globals: &RuntimeGlobals,
  compiler_options: &CompilerOptions,
) -> String {
  if runtime_globals == &RuntimeGlobals::EXPORTS {
    return runtime_variable_to_string(&RuntimeVariable::Exports, compiler_options);
  }

  if runtime_globals == &RuntimeGlobals::REQUIRE {
    return runtime_variable_to_string(&RuntimeVariable::Require, compiler_options);
  }

  if runtime_globals == &RuntimeGlobals::MODULE {
    return "module".to_string();
  }

  let name = runtime_globals_property_name(runtime_globals)
    .unwrap_or_else(|| panic!("runtime global {runtime_globals:?} cannot be rendered as string"));
  if REQUIRE_SCOPE_GLOBALS.contains(*runtime_globals) {
    let require = runtime_variable_to_string(&RuntimeVariable::Require, compiler_options);
    let mut result = String::with_capacity(require.len() + 1 + name.len());
    result.push_str(&require);
    result.push('.');
    result.push_str(name);
    return result;
  }
  if MODULE_GLOBALS.contains(*runtime_globals) {
    let mut result = String::with_capacity("module".len() + 1 + name.len());
    result.push_str("module");
    result.push('.');
    result.push_str(name);
    return result;
  }
  name.to_string()
}

/// Returns the raw runtime-global property name without any owner object.
///
/// For example, `RuntimeGlobals::DEFINE_PROPERTY_GETTERS` returns `Some("d")`,
/// and `RuntimeGlobals::MODULE_FACTORIES_ADD_ONLY` returns
/// `Some("m (add only)")`.
pub fn runtime_globals_property_name(runtime_globals: &RuntimeGlobals) -> Option<&'static str> {
  Some(match *runtime_globals {
    RuntimeGlobals::REQUIRE_SCOPE => "*",
    RuntimeGlobals::MODULE_ID => "id",
    RuntimeGlobals::MODULE_LOADED => "loaded",
    RuntimeGlobals::MODULE_CACHE => "c",
    RuntimeGlobals::ENSURE_CHUNK => "e",
    RuntimeGlobals::ENSURE_CHUNK_HANDLERS => "f",
    RuntimeGlobals::PUBLIC_PATH => "p",
    RuntimeGlobals::GET_CHUNK_SCRIPT_FILENAME => "u",
    RuntimeGlobals::GET_CHUNK_CSS_FILENAME => "k",
    RuntimeGlobals::LOAD_SCRIPT => "l",
    RuntimeGlobals::HAS_OWN_PROPERTY => "o",
    RuntimeGlobals::MODULE_FACTORIES_ADD_ONLY => "m (add only)",
    RuntimeGlobals::ON_CHUNKS_LOADED => "O",
    RuntimeGlobals::CHUNK_CALLBACK => "global chunk callback",
    RuntimeGlobals::MODULE_FACTORIES => "m",
    RuntimeGlobals::INTERCEPT_MODULE_EXECUTION => "i",
    RuntimeGlobals::HMR_DOWNLOAD_MANIFEST => "hmrM",
    RuntimeGlobals::HMR_DOWNLOAD_UPDATE_HANDLERS => "hmrC",
    RuntimeGlobals::HMR_INVALIDATE_MODULE_HANDLERS => "hmrI",
    RuntimeGlobals::HMR_MODULE_DATA => "hmrD",
    RuntimeGlobals::HMR_RUNTIME_STATE_PREFIX => "hmrS",
    RuntimeGlobals::GET_UPDATE_MANIFEST_FILENAME => "hmrF",
    RuntimeGlobals::GET_CHUNK_UPDATE_SCRIPT_FILENAME => "hu",
    RuntimeGlobals::GET_CHUNK_UPDATE_CSS_FILENAME => "hk",
    RuntimeGlobals::AMD_DEFINE => "amdD",
    RuntimeGlobals::AMD_OPTIONS => "amdO",
    RuntimeGlobals::EXTERNAL_INSTALL_CHUNK => "C",
    RuntimeGlobals::GET_FULL_HASH => "h",
    RuntimeGlobals::GLOBAL => "g",
    RuntimeGlobals::RETURN_EXPORTS_FROM_RUNTIME => "return-exports-from-runtime",
    RuntimeGlobals::INSTANTIATE_WASM => "v",
    RuntimeGlobals::ASYNC_MODULE => "a",
    RuntimeGlobals::ASYNC_MODULE_EXPORT_SYMBOL => "aE",
    RuntimeGlobals::BASE_URI => "b",
    RuntimeGlobals::STARTUP_ENTRYPOINT => "X",
    RuntimeGlobals::STARTUP_CHUNK_DEPENDENCIES => "x (chunk dependencies)",
    RuntimeGlobals::CREATE_SCRIPT_URL => "tu",
    RuntimeGlobals::CREATE_SCRIPT => "ts",
    RuntimeGlobals::GET_TRUSTED_TYPES_POLICY => "tt",
    RuntimeGlobals::DEFINE_PROPERTY_GETTERS => "d",
    RuntimeGlobals::ENTRY_MODULE_ID => "s",
    RuntimeGlobals::STARTUP_NO_DEFAULT => "x (no default handler)",
    RuntimeGlobals::ENSURE_CHUNK_INCLUDE_ENTRIES => "f (include entries)",
    RuntimeGlobals::STARTUP => "x",
    RuntimeGlobals::MAKE_NAMESPACE_OBJECT => "r",
    RuntimeGlobals::MAKE_DEFERRED_NAMESPACE_OBJECT => "z",
    RuntimeGlobals::MAKE_OPTIMIZED_DEFERRED_NAMESPACE_OBJECT => "zO",
    RuntimeGlobals::DEFERRED_MODULES_ASYNC_TRANSITIVE_DEPENDENCIES => "zT",
    RuntimeGlobals::DEFERRED_MODULES_ASYNC_TRANSITIVE_DEPENDENCIES_SYMBOL => "zS",
    RuntimeGlobals::COMPAT_GET_DEFAULT_EXPORT => "n",
    RuntimeGlobals::CREATE_FAKE_NAMESPACE_OBJECT => "t",
    RuntimeGlobals::ESM_MODULE_DECORATOR => "hmd",
    RuntimeGlobals::NODE_MODULE_DECORATOR => "nmd",
    RuntimeGlobals::SYSTEM_CONTEXT => "y",
    RuntimeGlobals::THIS_AS_EXPORTS => "top-level-this-exports",
    RuntimeGlobals::CURRENT_REMOTE_GET_SCOPE => "R",
    RuntimeGlobals::SHARE_SCOPE_MAP => "S",
    RuntimeGlobals::INITIALIZE_SHARING => "I",
    RuntimeGlobals::SCRIPT_NONCE => "nc",
    RuntimeGlobals::RELATIVE_URL => "U",
    RuntimeGlobals::CHUNK_NAME => "cn",
    RuntimeGlobals::RUNTIME_ID => "j",
    RuntimeGlobals::PREFETCH_CHUNK => "E",
    RuntimeGlobals::PREFETCH_CHUNK_HANDLERS => "F",
    RuntimeGlobals::PRELOAD_CHUNK => "G",
    RuntimeGlobals::PRELOAD_CHUNK_HANDLERS => "H",
    RuntimeGlobals::UNCAUGHT_ERROR_HANDLER => "oe",
    // rspack only
    RuntimeGlobals::RSPACK_VERSION => "rv",
    RuntimeGlobals::RSPACK_UNIQUE_ID => "ruid",
    RuntimeGlobals::HAS_CSS_MODULES => "has css modules",
    RuntimeGlobals::ASYNC_STARTUP => "asyncStartup",
    RuntimeGlobals::HAS_FETCH_PRIORITY => "has fetch priority",

    RuntimeGlobals::RSC_MANIFEST => "rscM",
    RuntimeGlobals::TO_BINARY => "tb",
    _ => return None,
  })
}

static RUNTIME_GLOBALS_BY_PROPERTY_NAME: LazyLock<FxHashMap<&'static str, RuntimeGlobals>> =
  LazyLock::new(|| {
    let mut map = FxHashMap::default();
    for (_, runtime_global) in RuntimeGlobals::all().iter_names() {
      if let Some(property_name) = runtime_globals_property_name(&runtime_global) {
        map.entry(property_name).or_insert(runtime_global);
      }
    }
    map.shrink_to_fit();
    map
  });

static RUNTIME_GLOBAL_LEXICAL_VARIABLE_MAP: LazyLock<FxHashMap<RuntimeGlobals, String>> =
  LazyLock::new(|| {
    let mut map = FxHashMap::default();
    for (_, runtime_global) in RuntimeGlobals::all().iter_names() {
      if let Some(property) = runtime_globals_property_name(&runtime_global) {
        let suffix = if property
          .chars()
          .all(|c| c == '_' || c == '$' || c.is_ascii_alphanumeric())
        {
          property.to_string()
        } else {
          let mut suffix = property
            .chars()
            .map(|c| {
              if c == '_' || c == '$' || c.is_ascii_alphanumeric() {
                c
              } else {
                '_'
              }
            })
            .collect::<String>()
            .trim_matches('_')
            .to_string();
          if suffix.is_empty() {
            suffix.push('x');
          }
          suffix
        };
        map.insert(runtime_global, format!("__var_{suffix}"));
      }
    }
    map.shrink_to_fit();
    map
  });

/// Finds the runtime global represented by a legacy require property name.
///
/// For example, `d` resolves to `RuntimeGlobals::DEFINE_PROPERTY_GETTERS`
/// and `nc` resolves to `RuntimeGlobals::SCRIPT_NONCE`.
pub fn runtime_globals_from_property_name(property_name: &str) -> Option<RuntimeGlobals> {
  RUNTIME_GLOBALS_BY_PROPERTY_NAME.get(property_name).copied()
}

/// Renders the lexical variable name for a runtime global.
///
/// For example, `RuntimeGlobals::DEFINE_PROPERTY_GETTERS` renders to `__var_d`.
pub fn runtime_globals_to_lexical_variable(
  runtime_globals: &RuntimeGlobals,
  _compiler_options: &CompilerOptions,
) -> String {
  RUNTIME_GLOBAL_LEXICAL_VARIABLE_MAP
    .get(runtime_globals)
    .unwrap_or_else(|| {
      panic!("runtime global {runtime_globals:?} cannot be rendered as lexical variable")
    })
    .clone()
}

/// Returns whether a runtime global can be rendered through require-scope forms.
///
/// For example, `RuntimeGlobals::DEFINE_PROPERTY_GETTERS` returns `true` and can
/// render as `__webpack_require__.d` or `__rspack_runtime.d`, while
/// `RuntimeGlobals::REQUIRE_SCOPE` returns `false`.
pub fn runtime_global_can_render_in_require_scope(runtime_globals: &RuntimeGlobals) -> bool {
  runtime_globals_property_name(runtime_globals).is_some()
    && RENDERABLE_REQUIRE_SCOPE_GLOBALS.contains(*runtime_globals)
}

/// Filters a runtime global set to globals that can be rendered as concrete
/// require-scope properties.
///
/// For example, `RuntimeGlobals::REQUIRE_SCOPE | RuntimeGlobals::PUBLIC_PATH`
/// renders only `RuntimeGlobals::PUBLIC_PATH`, because `REQUIRE_SCOPE` means the
/// broad `__webpack_require__.*` surface rather than one property.
pub fn renderable_require_scope_runtime_globals(runtime_globals: RuntimeGlobals) -> RuntimeGlobals {
  runtime_globals.intersection(*RENDERABLE_REQUIRE_SCOPE_GLOBALS)
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum RuntimeVariable {
  Require,
  /// The runtime chunk local proxy object captured by module factories.
  RuntimeProxy,
  /// The non-ESM async chunk payload field that installs the parent runtime
  /// proxy into the loaded chunk.
  ExternalRuntimeProxy,
  EsmId,
  EsmIds,
  EsmRuntime,
  /// The ESM async chunk export that installs the parent runtime proxy.
  EsmRuntimeProxy,
  Modules,
  ModuleCache,
  Module,
  Exports,
  StartupExec,
}

pub fn runtime_variable_to_template_name(runtime_variable: &RuntimeVariable) -> &'static str {
  match runtime_variable {
    RuntimeVariable::Require => "VAR_REQUIRE",
    RuntimeVariable::RuntimeProxy => "VAR_RUNTIME_PROXY",
    RuntimeVariable::ExternalRuntimeProxy => "VAR_EXTERNAL_RUNTIME_PROXY",
    RuntimeVariable::EsmId => "VAR_ESM_ID",
    RuntimeVariable::EsmIds => "VAR_ESM_IDS",
    RuntimeVariable::EsmRuntime => "VAR_ESM_RUNTIME",
    RuntimeVariable::EsmRuntimeProxy => "VAR_ESM_RUNTIME_PROXY",
    RuntimeVariable::Modules => "VAR_MODULES",
    RuntimeVariable::ModuleCache => "VAR_MODULE_CACHE",
    RuntimeVariable::Module => "VAR_MODULE",
    RuntimeVariable::Exports => "VAR_EXPORTS",
    RuntimeVariable::StartupExec => "VAR_STARTUP_EXEC",
  }
}

/// Renders a named runtime variable.
///
/// For example, `RuntimeVariable::Require` always renders to
/// `__webpack_require__`, while `RuntimeVariable::RuntimeProxy` renders to
/// `__rspack_runtime`.
pub fn runtime_variable_to_string(
  runtime_variable: &RuntimeVariable,
  compiler_options: &CompilerOptions,
) -> String {
  match (*runtime_variable, compiler_options.mode.is_production()) {
    (RuntimeVariable::RuntimeProxy, _) => "__rspack_runtime".to_string(),
    (RuntimeVariable::ExternalRuntimeProxy, _) => "installRuntime".to_string(),
    (RuntimeVariable::EsmRuntime, _) => "__rspack_esm_install_runtime_modules__".to_string(),
    (RuntimeVariable::EsmRuntimeProxy, _) => "__rspack_install_runtime__".to_string(),
    (RuntimeVariable::Require, _) => "__webpack_require__".to_string(),
    (RuntimeVariable::EsmId, _) => "__rspack_esm_id".to_string(),
    (RuntimeVariable::EsmIds, _) => "__rspack_esm_ids".to_string(),
    (RuntimeVariable::Modules, _) => "__webpack_modules__".to_string(),
    (RuntimeVariable::ModuleCache, _) => "__webpack_module_cache__".to_string(),
    (RuntimeVariable::Exports, _) => "__webpack_exports__".to_string(),
    (RuntimeVariable::Module, _) => "__webpack_module__".to_string(),
    (RuntimeVariable::StartupExec, _) => "__webpack_exec__".to_string(),
  }
}

type RuntimeGlobalMap = (
  FxHashMap<RuntimeGlobals, &'static str>,
  FxHashMap<&'static str, RuntimeGlobals>,
);

static RUNTIME_GLOBAL_MAP: LazyLock<RuntimeGlobalMap> = LazyLock::new(|| {
  let mut to_js_map = FxHashMap::default();
  let mut from_js_map = FxHashMap::default();

  for (name, value) in RuntimeGlobals::all().iter_names() {
    to_js_map.insert(value, name);
    from_js_map.insert(name, value);
  }

  to_js_map.shrink_to_fit();
  from_js_map.shrink_to_fit();
  (to_js_map, from_js_map)
});

impl RuntimeGlobals {
  pub fn to_names(&self) -> Vec<&'static str> {
    let mut res = vec![];

    for (item, js_name) in RUNTIME_GLOBAL_MAP.0.iter() {
      if self.contains(*item) {
        res.push(*js_name);
      }
    }
    res
  }
  pub fn from_names(names: &[String]) -> Self {
    let mut res = Self::empty();

    for name in names {
      if let Some(value) = RUNTIME_GLOBAL_MAP.1.get(name.as_str()) {
        res.insert(*value);
      }
    }
    res
  }
}
