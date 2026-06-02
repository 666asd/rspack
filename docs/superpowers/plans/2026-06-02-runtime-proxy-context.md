# Runtime Proxy Context Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement `experiments.runtimeMode: "webpack" | "rspack"` so runtime modules use lexical helper variables and normal modules read helpers from `__rspack_context`.

**Architecture:** Keep webpack mode as the existing default. In rspack mode, add context-aware RuntimeGlobals rendering, render runtime modules with lexical names, compute runtime chunk metadata for context exposure, and pass `__rspack_context` as the unchanged third module factory argument. Unsupported static `__webpack_require__.x` helper access and custom runtime modules produce compile-time errors.

**Tech Stack:** Rust (`rspack_core`, `rspack_plugin_javascript`, runtime plugins), TypeScript config plumbing (`packages/rspack`), NAPI raw options, Rspack config cases and unit tests.

---

## Scope Check

This plan implements one coherent runtime output mode. It intentionally excludes any `compatibility` mode, any `__webpack_require__.x` bridge, any async chunk install-runtime protocol, and custom runtime module support in rspack mode.

## File Structure

- `packages/rspack/src/config/types.ts`: public `experiments.runtimeMode` type.
- `packages/rspack/src/config/normalization.ts`: normalized option type.
- `packages/rspack/src/config/defaults.ts`: default to `"webpack"`.
- `packages/rspack/src/config/adapter.ts`: pass runtime mode to raw options.
- `crates/rspack_binding_api/src/raw_options/raw_experiments/mod.rs`: convert JS raw mode to core mode.
- `crates/rspack_core/src/options/experiments/mod.rs`: core `RuntimeMode` enum and `Experiments.runtime_mode`.
- `crates/rspack/src/builder/mod.rs`: native builder support.
- `crates/rspack_core/src/runtime_globals.rs`: legacy property names, context property names, lexical names, runtime variables.
- `crates/rspack_core/src/runtime_template.rs`: runtime render modes for module and runtime module code generation.
- `crates/rspack_core/src/dependency/runtime_requirements_dependency.rs`: add write mode and static unsupported-require-property mode.
- `crates/rspack_core/src/artifacts/runtime_proxy_metadata_artifact.rs`: metadata for context fields.
- `crates/rspack_core/src/artifacts/mod.rs`, `crates/rspack_core/src/compilation/mod.rs`, `crates/rspack_core/src/cache/memory.rs`: artifact registration.
- `crates/rspack_core/src/compilation/runtime_requirements/mod.rs`: collect runtime proxy metadata and reject custom runtime modules.
- `crates/rspack_core/src/compilation/code_generation/mod.rs`: render ordinary modules in rspack module mode.
- `crates/rspack_core/src/concatenated_module.rs`: render concatenated child modules in rspack module mode.
- `crates/rspack_core/src/compilation/create_hash/mod.rs`: render runtime modules in lexical mode for hashing.
- `crates/rspack_plugin_javascript/src/runtime.rs`: render runtime module lexical declarations and context fields.
- `crates/rspack_plugin_javascript/src/plugin/mod.rs`: create context and pass it into module factories.
- `crates/rspack_plugin_javascript/src/parser_plugin/api_plugin.rs`: parse API writes, require calls, and unsupported static require property access.
- `tests/rspack-test/configCases/runtime/runtime-mode-*`: focused config tests and snapshots.

---

### Task 1: Add `experiments.runtimeMode` Plumbing

**Files:**
- Modify: `packages/rspack/src/config/types.ts`
- Modify: `packages/rspack/src/config/normalization.ts`
- Modify: `packages/rspack/src/config/defaults.ts`
- Modify: `packages/rspack/src/config/adapter.ts`
- Modify: `crates/rspack_binding_api/src/raw_options/raw_experiments/mod.rs`
- Modify: `crates/rspack_core/src/options/experiments/mod.rs`
- Modify: `crates/rspack/src/builder/mod.rs`
- Test: `tests/rspack-test/configCases/runtime/runtime-mode-option`

- [ ] **Step 1: Add the failing config case**

Create `tests/rspack-test/configCases/runtime/runtime-mode-option/rspack.config.js`:

```js
module.exports = {
  experiments: {
    runtimeMode: "rspack"
  }
};
```

Create `tests/rspack-test/configCases/runtime/runtime-mode-option/index.js`:

```js
export default 1;
```

- [ ] **Step 2: Run the focused case and confirm it fails before plumbing**

Run:

```bash
cd tests/rspack-test && pnpm run test -t "configCases/runtime/runtime-mode-option"
```

Expected: FAIL because `runtimeMode` is unknown or dropped before reaching Rust.

- [ ] **Step 3: Add the public TypeScript option**

In `packages/rspack/src/config/types.ts`, add this field to `export type Experiments`:

```ts
  /**
   * Selects the runtime helper organization mode.
   *
   * - `"webpack"` keeps the webpack-compatible `__webpack_require__.x` output.
   * - `"rspack"` renders runtime modules as lexical variables and exposes module-visible helpers on `__rspack_context`.
   *
   * @experimental
   * @default "webpack"
   */
  runtimeMode?: "webpack" | "rspack";
```

In `packages/rspack/src/config/normalization.ts`, add to `ExperimentsNormalized`:

```ts
  runtimeMode?: "webpack" | "rspack";
```

In `packages/rspack/src/config/defaults.ts`, add inside `applyExperimentsDefaults`:

```ts
  D(experiments, "runtimeMode", "webpack");
```

- [ ] **Step 4: Pass the raw option to Rust**

In `packages/rspack/src/config/adapter.ts`, ensure the returned raw experiments object includes:

```ts
runtimeMode: experiments.runtimeMode,
```

In `crates/rspack_core/src/options/experiments/mod.rs`, add:

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RuntimeMode {
  #[default]
  Webpack,
  Rspack,
}
```

Then add this field to `Experiments`:

```rust
pub runtime_mode: RuntimeMode,
```

In `crates/rspack_binding_api/src/raw_options/raw_experiments/mod.rs`, add a raw field:

```rust
#[napi(ts_type = "\"webpack\" | \"rspack\"")]
pub runtime_mode: Option<String>,
```

Convert it with an explicit match:

```rust
let runtime_mode = match value.runtime_mode.as_deref() {
  Some("rspack") => RuntimeMode::Rspack,
  Some("webpack") | None => RuntimeMode::Webpack,
  Some(other) => panic!("unsupported experiments.runtimeMode: {other}"),
};
```

Set `runtime_mode` in `Experiments`.

- [ ] **Step 5: Update the Rust builder**

In `crates/rspack/src/builder/mod.rs`, add an optional builder field:

```rust
runtime_mode: Option<RuntimeMode>,
```

Update `impl From<Experiments> for ExperimentsBuilder`, `impl From<&mut ExperimentsBuilder> for ExperimentsBuilder`, and the final builder conversion so omitted values default to `RuntimeMode::Webpack`.

- [ ] **Step 6: Build JS and run the option case**

Run:

```bash
pnpm run build:js
cd tests/rspack-test && pnpm run test -t "configCases/runtime/runtime-mode-option"
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add packages/rspack/src/config/types.ts packages/rspack/src/config/normalization.ts packages/rspack/src/config/defaults.ts packages/rspack/src/config/adapter.ts crates/rspack_binding_api/src/raw_options/raw_experiments/mod.rs crates/rspack_core/src/options/experiments/mod.rs crates/rspack/src/builder/mod.rs tests/rspack-test/configCases/runtime/runtime-mode-option
git commit -m "feat: add runtime mode experiment"
```

---

### Task 2: Add Runtime Global Context And Lexical Render Helpers

**Files:**
- Modify: `crates/rspack_core/src/runtime_globals.rs`
- Modify: `crates/rspack_core/src/runtime_template.rs`
- Test: `tests/rspack-test/configCases/runtime/runtime-mode-module-rendering`

- [ ] **Step 1: Add a failing output case for context helper keys**

Create `tests/rspack-test/configCases/runtime/runtime-mode-module-rendering/rspack.config.js`:

```js
module.exports = {
  experiments: {
    runtimeMode: "rspack"
  },
  optimization: {
    concatenateModules: false
  }
};
```

Create `tests/rspack-test/configCases/runtime/runtime-mode-module-rendering/index.js`:

```js
import { value } from "./lib";
export { value };
```

Create `tests/rspack-test/configCases/runtime/runtime-mode-module-rendering/lib.js`:

```js
export const value = 42;
```

Create `tests/rspack-test/configCases/runtime/runtime-mode-module-rendering/after.js`:

```js
const fs = require("fs");
const path = require("path");

const source = fs.readFileSync(path.resolve(__dirname, "dist/main.js"), "utf-8");

expect(source).toContain("__rspack_context.d");
expect(source).toContain("__rspack_context.ns");
expect(source).not.toContain("__webpack_require__.d(__webpack_exports__");
expect(source).not.toContain("__webpack_require__.r(__webpack_exports__");
```

- [ ] **Step 2: Run the case and confirm current output fails**

Run:

```bash
pnpm run build:js
pnpm run build:binding:dev
cd tests/rspack-test && pnpm run test -t "configCases/runtime/runtime-mode-module-rendering"
```

Expected: FAIL because module code still renders `__webpack_require__.d` and `__webpack_require__.r`.

- [ ] **Step 3: Add centralized property helpers**

In `crates/rspack_core/src/runtime_globals.rs`, extract the existing `match *runtime_globals` body from `runtime_globals_to_string` into:

```rust
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
    RuntimeGlobals::DEFINE_PROPERTY_GETTERS => "d",
    RuntimeGlobals::MAKE_NAMESPACE_OBJECT => "r",
    RuntimeGlobals::COMPAT_GET_DEFAULT_EXPORT => "n",
    RuntimeGlobals::CREATE_FAKE_NAMESPACE_OBJECT => "t",
    RuntimeGlobals::SCRIPT_NONCE => "nc",
    // Keep the rest of the existing runtime_globals_to_string match arms here unchanged.
    _ => return None,
  })
}
```

Then make `runtime_globals_to_string` call `runtime_globals_property_name(runtime_globals).expect("runtime global should have property name")`.

Add context and lexical helpers:

```rust
pub fn runtime_globals_context_property_name(runtime_globals: &RuntimeGlobals) -> Option<&'static str> {
  if runtime_globals == &RuntimeGlobals::MAKE_NAMESPACE_OBJECT {
    return Some("ns");
  }
  runtime_globals_property_name(runtime_globals)
}

pub fn runtime_globals_to_lexical_name(runtime_globals: &RuntimeGlobals) -> Option<&'static str> {
  Some(match *runtime_globals {
    RuntimeGlobals::DEFINE_PROPERTY_GETTERS => "definePropertyGetters",
    RuntimeGlobals::MAKE_NAMESPACE_OBJECT => "makeNamespaceObject",
    RuntimeGlobals::COMPAT_GET_DEFAULT_EXPORT => "compatGetDefaultExport",
    RuntimeGlobals::CREATE_FAKE_NAMESPACE_OBJECT => "createFakeNamespaceObject",
    RuntimeGlobals::SCRIPT_NONCE => "scriptNonce",
    // Add every require-scope RuntimeGlobals used by runtime modules with camelCase names.
    _ => return None,
  })
}
```

Add a helper for renderable require-scope filtering:

```rust
pub fn renderable_require_scope_runtime_globals(runtime_globals: RuntimeGlobals) -> RuntimeGlobals {
  runtime_globals.intersection(*REQUIRE_SCOPE_GLOBALS).difference(RuntimeGlobals::REQUIRE_SCOPE)
}
```

- [ ] **Step 4: Add render modes to templates**

In `crates/rspack_core/src/runtime_template.rs`, add:

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RuntimeGlobalRenderMode {
  #[default]
  Webpack,
  RspackModule,
  RspackRuntimeModule,
}
```

Add `render_mode: RuntimeGlobalRenderMode` to `ModuleCodeTemplate` and `RuntimeCodeTemplate`.

Change constructors to receive the mode:

```rust
pub fn create_module_code_template(&self, render_mode: RuntimeGlobalRenderMode) -> ModuleCodeTemplate
pub fn create_runtime_code_template<'a>(&'a self, render_mode: RuntimeGlobalRenderMode) -> RuntimeCodeTemplate<'a>
```

Update call sites initially with `RuntimeGlobalRenderMode::Webpack` so behavior is unchanged until later tasks opt into rspack mode.

Implement one renderer:

```rust
fn render_runtime_globals_by_mode(
  runtime_globals: &RuntimeGlobals,
  render_mode: RuntimeGlobalRenderMode,
  compiler_options: &CompilerOptions,
) -> String {
  match render_mode {
    RuntimeGlobalRenderMode::Webpack => runtime_globals_to_string(runtime_globals, compiler_options),
    RuntimeGlobalRenderMode::RspackModule if runtime_globals == &RuntimeGlobals::REQUIRE => {
      "__rspack_context.r".to_string()
    }
    RuntimeGlobalRenderMode::RspackModule if REQUIRE_SCOPE_GLOBALS.contains(*runtime_globals) => {
      let property = runtime_globals_context_property_name(runtime_globals)
        .expect("runtime global should have context property");
      format!("__rspack_context{}", property_access([property], 0))
    }
    RuntimeGlobalRenderMode::RspackRuntimeModule if REQUIRE_SCOPE_GLOBALS.contains(*runtime_globals) => {
      runtime_globals_to_lexical_name(runtime_globals)
        .expect("runtime global should have lexical name")
        .to_string()
    }
    RuntimeGlobalRenderMode::RspackModule | RuntimeGlobalRenderMode::RspackRuntimeModule => {
      runtime_globals_to_string(runtime_globals, compiler_options)
    }
  }
}
```

Then make both `ModuleCodeTemplate::render_runtime_globals*` and `RuntimeCodeTemplate::render_runtime_globals` use this renderer.

- [ ] **Step 5: Run Rust formatting and focused case**

Run:

```bash
pnpm run format:rs
pnpm run build:binding:dev
cd tests/rspack-test && pnpm run test -t "configCases/runtime/runtime-mode-module-rendering"
```

Expected: still FAIL until Task 3 enables rspack module mode in code generation, but Rust builds.

- [ ] **Step 6: Commit**

```bash
git add crates/rspack_core/src/runtime_globals.rs crates/rspack_core/src/runtime_template.rs tests/rspack-test/configCases/runtime/runtime-mode-module-rendering
git commit -m "feat: add runtime global context rendering"
```

---

### Task 3: Render Ordinary Modules Through `__rspack_context`

**Files:**
- Modify: `crates/rspack_core/src/compilation/code_generation/mod.rs`
- Modify: `crates/rspack_core/src/concatenated_module.rs`
- Test: `tests/rspack-test/configCases/runtime/runtime-mode-module-rendering`

- [ ] **Step 1: Enable rspack module render mode for ordinary code generation**

In `crates/rspack_core/src/compilation/code_generation/mod.rs`, where module code generation creates a module code template, choose:

```rust
let render_mode = if this.options.experiments.runtime_mode == RuntimeMode::Rspack {
  RuntimeGlobalRenderMode::RspackModule
} else {
  RuntimeGlobalRenderMode::Webpack
};
let mut runtime_template = this.runtime_template.create_module_code_template(render_mode);
```

Import `RuntimeMode` and `RuntimeGlobalRenderMode`.

- [ ] **Step 2: Enable rspack module render mode for concatenated modules**

In `crates/rspack_core/src/concatenated_module.rs`, apply the same render mode selection for child module templates:

```rust
let render_mode = if compilation.options.experiments.runtime_mode == RuntimeMode::Rspack {
  RuntimeGlobalRenderMode::RspackModule
} else {
  RuntimeGlobalRenderMode::Webpack
};
let mut runtime_template = compilation.runtime_template.create_module_code_template(render_mode);
```

- [ ] **Step 3: Run the focused module rendering case**

Run:

```bash
pnpm run build:binding:dev
cd tests/rspack-test && pnpm run test -t "configCases/runtime/runtime-mode-module-rendering"
```

Expected: PASS for module output assertions.

- [ ] **Step 4: Commit**

```bash
git add crates/rspack_core/src/compilation/code_generation/mod.rs crates/rspack_core/src/concatenated_module.rs tests/rspack-test/configCases/runtime/runtime-mode-module-rendering
git commit -m "feat: render modules with rspack runtime context"
```

---

### Task 4: Add Runtime Proxy Metadata Artifact

**Files:**
- Create: `crates/rspack_core/src/artifacts/runtime_proxy_metadata_artifact.rs`
- Modify: `crates/rspack_core/src/artifacts/mod.rs`
- Modify: `crates/rspack_core/src/compilation/mod.rs`
- Modify: `crates/rspack_core/src/cache/memory.rs`
- Modify: `crates/rspack_core/src/compilation/runtime_requirements/mod.rs`
- Test: `tests/rspack-test/configCases/runtime/runtime-mode-custom-runtime-module-error`

- [ ] **Step 1: Add the metadata artifact**

Create `crates/rspack_core/src/artifacts/runtime_proxy_metadata_artifact.rs`:

```rust
use std::ops::{Deref, DerefMut};

use rustc_hash::FxHashMap;

use crate::{ArtifactExt, ChunkUkey, RuntimeGlobals, incremental::IncrementalPasses};

#[derive(Debug, Default, Clone)]
pub struct RuntimeProxyMetadata {
  pub module_proxy_requirements: RuntimeGlobals,
  pub runtime_module_requirements: RuntimeGlobals,
  pub context_setter_fields: RuntimeGlobals,
  pub hook_exposed_requirements: RuntimeGlobals,
  pub has_custom_runtime_module: bool,
}

impl RuntimeProxyMetadata {
  pub fn lexical_fields(&self) -> RuntimeGlobals {
    self
      .runtime_module_requirements
      .union(self.module_proxy_requirements)
      .union(self.context_setter_fields)
      .union(self.hook_exposed_requirements)
  }

  pub fn context_fields(&self) -> RuntimeGlobals {
    self
      .module_proxy_requirements
      .union(self.context_setter_fields)
      .union(self.hook_exposed_requirements)
  }
}

#[derive(Debug, Default, Clone)]
pub struct RuntimeProxyMetadataArtifact(FxHashMap<ChunkUkey, RuntimeProxyMetadata>);

impl ArtifactExt for RuntimeProxyMetadataArtifact {
  const PASS: IncrementalPasses = IncrementalPasses::CHUNKS_RUNTIME_REQUIREMENTS;
}

impl Deref for RuntimeProxyMetadataArtifact {
  type Target = FxHashMap<ChunkUkey, RuntimeProxyMetadata>;

  fn deref(&self) -> &Self::Target {
    &self.0
  }
}

impl DerefMut for RuntimeProxyMetadataArtifact {
  fn deref_mut(&mut self) -> &mut Self::Target {
    &mut self.0
  }
}
```

- [ ] **Step 2: Register the artifact**

In `crates/rspack_core/src/artifacts/mod.rs`, export:

```rust
mod runtime_proxy_metadata_artifact;
pub use runtime_proxy_metadata_artifact::{RuntimeProxyMetadata, RuntimeProxyMetadataArtifact};
```

In `crates/rspack_core/src/compilation/mod.rs`, add:

```rust
pub runtime_proxy_metadata_artifact: StealCell<RuntimeProxyMetadataArtifact>,
```

Initialize it in `Compilation::new`:

```rust
runtime_proxy_metadata_artifact: StealCell::new(Default::default()),
```

In `crates/rspack_core/src/cache/memory.rs`, recover it next to other runtime requirement artifacts:

```rust
recover_artifact(
  incremental,
  &mut compilation.runtime_proxy_metadata_artifact,
  &mut old_compilation.runtime_proxy_metadata_artifact,
);
```

- [ ] **Step 3: Add custom runtime module error case**

Create `tests/rspack-test/configCases/runtime/runtime-mode-custom-runtime-module-error/rspack.config.js`:

```js
const { RuntimeModule } = require("@rspack/core");

class CustomRuntimeModule extends RuntimeModule {
  constructor() {
    super("custom runtime module");
  }

  generate() {
    return "__webpack_require__.customRuntimeModuleTouched = true;";
  }
}

class CustomRuntimeModulePlugin {
  apply(compiler) {
    compiler.hooks.compilation.tap("CustomRuntimeModulePlugin", compilation => {
      compilation.hooks.additionalTreeRuntimeRequirements.tap(
        "CustomRuntimeModulePlugin",
        chunk => {
          compilation.addRuntimeModule(chunk, new CustomRuntimeModule());
        }
      );
    });
  }
}

module.exports = {
  experiments: {
    runtimeMode: "rspack"
  },
  plugins: [new CustomRuntimeModulePlugin()]
};
```

Create `tests/rspack-test/configCases/runtime/runtime-mode-custom-runtime-module-error/index.js`:

```js
export default 1;
```

Create `tests/rspack-test/configCases/runtime/runtime-mode-custom-runtime-module-error/test.config.js`:

```js
module.exports = {
  error: true
};
```

- [ ] **Step 4: Compute metadata and reject custom runtime modules**

In `crates/rspack_core/src/compilation/runtime_requirements/mod.rs`, after `compilation.runtime_modules = runtime_modules;`, clear metadata and skip in webpack mode:

```rust
compilation.runtime_proxy_metadata_artifact.clear();
if compilation.options.experiments.runtime_mode != RuntimeMode::Rspack {
  logger.time_end(start);
  return Ok(());
}
```

Then for each runtime entry, collect metadata:

```rust
for entry_ukey in &entries {
  let mut metadata = RuntimeProxyMetadata::default();
  let entry = compilation
    .build_chunk_graph_artifact
    .chunk_by_ukey
    .expect_get(entry_ukey);

  for chunk_ukey in entry
    .get_all_referenced_chunks(&compilation.build_chunk_graph_artifact.chunk_group_by_ukey)
    .iter()
  {
    let chunk = compilation
      .build_chunk_graph_artifact
      .chunk_by_ukey
      .expect_get(chunk_ukey);

    for mid in compilation
      .build_chunk_graph_artifact
      .chunk_graph
      .get_chunk_modules_identifier(chunk_ukey)
    {
      if let Some(runtime_requirements) =
        ChunkGraph::get_module_runtime_requirements(compilation, *mid, chunk.runtime())
      {
        metadata
          .module_proxy_requirements
          .insert(renderable_require_scope_runtime_globals(*runtime_requirements));
      }
    }

    for runtime_module_id in compilation
      .build_chunk_graph_artifact
      .chunk_graph
      .get_chunk_runtime_modules_iterable(chunk_ukey)
    {
      let runtime_module = &compilation.runtime_modules[runtime_module_id];
      metadata.has_custom_runtime_module |= runtime_module.get_custom_source().is_some()
        || runtime_module.get_constructor_name() == "RuntimeModuleFromJs";
      metadata.runtime_module_requirements.insert(
        renderable_require_scope_runtime_globals(
          runtime_module.additional_runtime_requirements(compilation),
        ),
      );
    }

    metadata.hook_exposed_requirements.insert(
      renderable_require_scope_runtime_globals(
        *ChunkGraph::get_tree_runtime_requirements(compilation, chunk_ukey),
      ),
    );
  }

  if metadata.has_custom_runtime_module {
    return Err(error!("experiments.runtimeMode: \"rspack\" does not support custom runtime modules because __webpack_require__.x helper access is not exposed."));
  }

  compilation
    .runtime_proxy_metadata_artifact
    .insert(*entry_ukey, metadata);
}
```

Use the project’s existing diagnostic/error helper imports in this file.

- [ ] **Step 5: Run focused tests**

Run:

```bash
pnpm run build:binding:dev
cd tests/rspack-test && pnpm run test -t "configCases/runtime/runtime-mode-custom-runtime-module-error|configCases/runtime/runtime-mode-module-rendering"
```

Expected: custom runtime case fails compilation with a clear error, module rendering still passes.

- [ ] **Step 6: Commit**

```bash
git add crates/rspack_core/src/artifacts/runtime_proxy_metadata_artifact.rs crates/rspack_core/src/artifacts/mod.rs crates/rspack_core/src/compilation/mod.rs crates/rspack_core/src/cache/memory.rs crates/rspack_core/src/compilation/runtime_requirements/mod.rs tests/rspack-test/configCases/runtime/runtime-mode-custom-runtime-module-error
git commit -m "feat: collect rspack runtime context metadata"
```

---

### Task 5: Render Runtime Modules With Lexical Runtime Globals

**Files:**
- Modify: `crates/rspack_core/src/compilation/create_hash/mod.rs`
- Modify: `crates/rspack_plugin_javascript/src/runtime.rs`
- Test: `tests/rspack-test/configCases/runtime/runtime-mode-module-rendering/after.js`

- [ ] **Step 1: Extend output assertions for lexical runtime modules**

Append to `tests/rspack-test/configCases/runtime/runtime-mode-module-rendering/after.js`:

```js
expect(source).toContain("var definePropertyGetters");
expect(source).toContain("var makeNamespaceObject");
expect(source).not.toContain("__webpack_require__.d =");
expect(source).not.toContain("__webpack_require__.r =");
```

- [ ] **Step 2: Enable lexical mode for runtime module code generation in hashing**

In `crates/rspack_core/src/compilation/create_hash/mod.rs`, inside runtime module code generation, choose:

```rust
let render_mode = if compilation.options.experiments.runtime_mode == RuntimeMode::Rspack {
  RuntimeGlobalRenderMode::RspackRuntimeModule
} else {
  RuntimeGlobalRenderMode::Webpack
};
let runtime_template = compilation.runtime_template.create_module_code_template(render_mode);
```

Use the existing mutability pattern required by the surrounding function.

- [ ] **Step 3: Enable lexical mode for runtime module rendering**

In `crates/rspack_plugin_javascript/src/runtime.rs`, wherever `render_runtime_modules` regenerates a runtime module source with a local `ModuleCodeTemplate`, use the same render mode selection:

```rust
let render_mode = if compilation.options.experiments.runtime_mode == RuntimeMode::Rspack {
  RuntimeGlobalRenderMode::RspackRuntimeModule
} else {
  RuntimeGlobalRenderMode::Webpack
};
let mut runtime_template = compilation.runtime_template.create_module_code_template(render_mode);
```

- [ ] **Step 4: Run focused case**

Run:

```bash
pnpm run build:binding:dev
cd tests/rspack-test && pnpm run test -t "configCases/runtime/runtime-mode-module-rendering"
```

Expected: PASS for lexical assertions.

- [ ] **Step 5: Commit**

```bash
git add crates/rspack_core/src/compilation/create_hash/mod.rs crates/rspack_plugin_javascript/src/runtime.rs tests/rspack-test/configCases/runtime/runtime-mode-module-rendering/after.js
git commit -m "feat: render runtime modules with lexical globals"
```

---

### Task 6: Emit `__rspack_context` And Pass It To Module Factories

**Files:**
- Modify: `crates/rspack_core/src/runtime_globals.rs`
- Modify: `crates/rspack_plugin_javascript/src/runtime.rs`
- Modify: `crates/rspack_plugin_javascript/src/plugin/mod.rs`
- Test: `tests/rspack-test/configCases/runtime/runtime-mode-module-rendering/after.js`
- Test: `tests/rspack-test/configCases/runtime/runtime-mode-async-chunk`

- [ ] **Step 1: Add runtime variable for context**

In `crates/rspack_core/src/runtime_globals.rs`, add:

```rust
RuntimeContext,
```

to `RuntimeVariable`, and render it as:

```rust
RuntimeVariable::RuntimeContext => "__rspack_context".to_string(),
```

- [ ] **Step 2: Render context declarations and fields**

In `crates/rspack_plugin_javascript/src/runtime.rs`, add helper functions:

```rust
pub fn render_runtime_context_declarations(
  compilation: &Compilation,
  chunk_ukey: &ChunkUkey,
  runtime_template: &RuntimeCodeTemplate<'_>,
) -> Option<BoxSource> {
  if compilation.options.experiments.runtime_mode != RuntimeMode::Rspack {
    return None;
  }
  let metadata = compilation.runtime_proxy_metadata_artifact.get(chunk_ukey)?;
  let lexical_fields = metadata.lexical_fields();
  let context_fields = metadata.context_fields();
  if lexical_fields.is_empty() && context_fields.is_empty() {
    return None;
  }
  let mut source = String::new();
  for runtime_global in lexical_fields.iter() {
    let lexical = runtime_globals_to_lexical_name(&runtime_global)
      .expect("runtime global should have lexical name");
    source.push_str(&format!("var {lexical};\n"));
  }
  let context = runtime_template.render_runtime_variable(&RuntimeVariable::RuntimeContext);
  let require = runtime_template.render_runtime_variable(&RuntimeVariable::Require);
  source.push_str(&format!("var {context} = {{ r: {require} }};\n"));
  Some(RawStringSource::from(source).boxed())
}
```

Add field rendering after runtime module sources:

```rust
pub fn render_runtime_context_fields(
  compilation: &Compilation,
  chunk_ukey: &ChunkUkey,
  runtime_template: &RuntimeCodeTemplate<'_>,
) -> Option<BoxSource> {
  if compilation.options.experiments.runtime_mode != RuntimeMode::Rspack {
    return None;
  }
  let metadata = compilation.runtime_proxy_metadata_artifact.get(chunk_ukey)?;
  let context_fields = metadata.context_fields();
  if context_fields.is_empty() {
    return None;
  }
  let mut source = String::new();
  let context = runtime_template.render_runtime_variable(&RuntimeVariable::RuntimeContext);
  for runtime_global in context_fields.iter() {
    let key = runtime_globals_context_property_name(&runtime_global)
      .expect("runtime global should have context property");
    let access = property_access([key], 0);
    let lexical = runtime_globals_to_lexical_name(&runtime_global)
      .expect("runtime global should have lexical name");
    if metadata.context_setter_fields.contains(runtime_global) {
      source.push_str(&format!(
        "Object.defineProperty({context}, {key:?}, {{ configurable: true, enumerable: true, get: function() {{ return {lexical}; }}, set: function(value) {{ {lexical} = value; }} }});\n"
      ));
    } else {
      source.push_str(&format!("{context}{access} = {lexical};\n"));
    }
  }
  Some(RawStringSource::from(source).boxed())
}
```

Use `json_stringify_str` instead of `{key:?}` if the local file already imports that helper.

- [ ] **Step 3: Place context declarations around runtime modules**

In `render_runtime_modules`, add declarations before runtime module sources:

```rust
if let Some(declarations) =
  render_runtime_context_declarations(compilation, chunk_ukey, runtime_template)
{
  sources.add(declarations);
}
```

Add fields after runtime module sources:

```rust
if let Some(fields) = render_runtime_context_fields(compilation, chunk_ukey, runtime_template) {
  sources.add(fields);
}
```

- [ ] **Step 4: Pass context into module factories**

In `crates/rspack_plugin_javascript/src/plugin/mod.rs`, update `render_require` so the factory third argument is:

```rust
let module_factory_require = if compilation.options.experiments.runtime_mode == RuntimeMode::Rspack {
  runtime_template.render_runtime_variable(&RuntimeVariable::RuntimeContext)
} else {
  runtime_template.render_runtime_globals(&RuntimeGlobals::REQUIRE)
};
```

Use `module_factory_require` in:

- the normal factory call,
- the `THIS_AS_EXPORTS` call,
- `execOptions.require` and `execOptions.factory.call(...)` for `INTERCEPT_MODULE_EXECUTION`.

- [ ] **Step 5: Extend focused assertions**

Append to `tests/rspack-test/configCases/runtime/runtime-mode-module-rendering/after.js`:

```js
expect(source).toContain("var __rspack_context = { r: __webpack_require__ };");
expect(source).toContain("__rspack_context.d = definePropertyGetters");
expect(source).toContain("__rspack_context.ns = makeNamespaceObject");
expect(source).toContain("module.exports, __rspack_context");
```

- [ ] **Step 6: Add async chunk case**

Create `tests/rspack-test/configCases/runtime/runtime-mode-async-chunk/rspack.config.js`:

```js
module.exports = {
  experiments: {
    runtimeMode: "rspack"
  },
  optimization: {
    concatenateModules: false
  }
};
```

Create `tests/rspack-test/configCases/runtime/runtime-mode-async-chunk/index.js`:

```js
export function load() {
  return import("./lazy").then(mod => mod.value);
}
```

Create `tests/rspack-test/configCases/runtime/runtime-mode-async-chunk/lazy.js`:

```js
export const value = 7;
```

Create `tests/rspack-test/configCases/runtime/runtime-mode-async-chunk/after.js`:

```js
const fs = require("fs");
const path = require("path");

const files = fs.readdirSync(path.resolve(__dirname, "dist"));
const main = fs.readFileSync(path.resolve(__dirname, "dist/main.js"), "utf-8");
const asyncFile = files.find(file => file !== "main.js" && file.endsWith(".js"));
const asyncSource = fs.readFileSync(path.resolve(__dirname, "dist", asyncFile), "utf-8");

expect(main).toContain("module.exports, __rspack_context");
expect(main).toContain("var __rspack_context = { r: __webpack_require__ };");
expect(asyncSource).toContain("__rspack_context.d");
expect(asyncSource).not.toContain("__rspack_install_runtime");
expect(asyncSource).not.toContain("__webpack_require__.d");
```

- [ ] **Step 7: Run focused cases**

Run:

```bash
pnpm run build:binding:dev
cd tests/rspack-test && pnpm run test -t "configCases/runtime/runtime-mode-module-rendering|configCases/runtime/runtime-mode-async-chunk"
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/rspack_core/src/runtime_globals.rs crates/rspack_plugin_javascript/src/runtime.rs crates/rspack_plugin_javascript/src/plugin/mod.rs tests/rspack-test/configCases/runtime/runtime-mode-module-rendering tests/rspack-test/configCases/runtime/runtime-mode-async-chunk
git commit -m "feat: emit rspack runtime context"
```

---

### Task 7: Parse Runtime API Writes And Unsupported Static Require Properties

**Files:**
- Modify: `crates/rspack_core/src/dependency/runtime_requirements_dependency.rs`
- Modify: `crates/rspack_plugin_javascript/src/parser_plugin/api_plugin.rs`
- Test: `tests/rspack-test/configCases/runtime/runtime-mode-context-setter`
- Test: `tests/rspack-test/configCases/runtime/runtime-mode-unsupported-require-property`

- [ ] **Step 1: Extend dependency modes**

In `crates/rspack_core/src/dependency/runtime_requirements_dependency.rs`, change the enum to:

```rust
pub enum RuntimeRequirementsDependencyMode {
  #[default]
  Normal,
  Call,
  AddOnly,
  Write,
  UnsupportedRequireProperty,
}
```

Add constructors:

```rust
pub fn write(range: DependencyRange, runtime_requirements: RuntimeGlobals) -> Self {
  Self {
    range,
    runtime_requirements,
    mode: RuntimeRequirementsDependencyMode::Write,
  }
}

pub fn unsupported_require_property(range: DependencyRange, runtime_requirements: RuntimeGlobals) -> Self {
  Self {
    range,
    runtime_requirements,
    mode: RuntimeRequirementsDependencyMode::UnsupportedRequireProperty,
  }
}
```

In `DependencyTemplate::render`, handle `Write`:

```rust
if matches!(dep.mode, RuntimeRequirementsDependencyMode::Write) {
  code_generatable_context
    .runtime_template
    .runtime_requirements_mut()
    .insert(dep.runtime_requirements);
  let content = code_generatable_context
    .runtime_template
    .render_runtime_globals(&dep.runtime_requirements);
  code_generatable_context
    .runtime_template
    .data
    .get_or_insert_default::<CodeGenerationRuntimeRequirementsWrite>()
    .insert(dep.runtime_requirements);
  source.replace(dep.range.start, dep.range.end, content, None);
  return;
}
```

Store write metadata through `TemplateContext.data`. Add this data type in `crates/rspack_core/src/dependency/runtime_requirements_dependency.rs` or a nearby core module exported by `rspack_core`:

```rust
#[derive(Debug, Default, Clone)]
pub struct CodeGenerationRuntimeRequirementsWrite {
  pub runtime_requirements: RuntimeGlobals,
}
```

- [ ] **Step 2: Record unsupported require property as a parser error**

In `RuntimeRequirementsDependencyTemplate::render`, for `UnsupportedRequireProperty`, render `undefined` so replacement still has a deterministic output if code generation continues after parser diagnostics:

```rust
if matches!(dep.mode, RuntimeRequirementsDependencyMode::UnsupportedRequireProperty) {
  source.replace(dep.range.start, dep.range.end, "undefined", None);
  return;
}
```

The actual compilation error is added in APIPlugin in Step 5 with `parser.add_error(...)`.

- [ ] **Step 3: Add setter case**

Create `tests/rspack-test/configCases/runtime/runtime-mode-context-setter/rspack.config.js`:

```js
module.exports = {
  experiments: {
    runtimeMode: "rspack"
  }
};
```

Create `tests/rspack-test/configCases/runtime/runtime-mode-context-setter/index.js`:

```js
__webpack_nonce__ = "nonce-value";
export const value = __webpack_nonce__;
```

Create `tests/rspack-test/configCases/runtime/runtime-mode-context-setter/after.js`:

```js
const fs = require("fs");
const path = require("path");

const source = fs.readFileSync(path.resolve(__dirname, "dist/main.js"), "utf-8");

expect(source).toContain("__rspack_context.nc");
expect(source).toContain("Object.defineProperty(__rspack_context, \"nc\"");
expect(source).toContain("set: function(value)");
expect(source).toContain("scriptNonce = value");
expect(source).not.toContain("__webpack_require__.nc");
```

- [ ] **Step 4: Add unsupported static require property case**

Create `tests/rspack-test/configCases/runtime/runtime-mode-unsupported-require-property/rspack.config.js`:

```js
module.exports = {
  experiments: {
    runtimeMode: "rspack"
  }
};
```

Create `tests/rspack-test/configCases/runtime/runtime-mode-unsupported-require-property/index.js`:

```js
const getter = __webpack_require__.d;
export { getter };
```

Create `tests/rspack-test/configCases/runtime/runtime-mode-unsupported-require-property/test.config.js`:

```js
module.exports = {
  error: true
};
```

- [ ] **Step 5: Update APIPlugin parsing**

In `crates/rspack_plugin_javascript/src/parser_plugin/api_plugin.rs`:

For `API_NONCE` assignment, add:

```rust
fn assign(
  &self,
  parser: &mut JavascriptParser,
  expr: &swc_core::ecma::ast::AssignExpr,
  for_name: &str,
) -> Option<bool> {
  if for_name == API_NONCE {
    parser.add_presentational_dependency(Box::new(RuntimeRequirementsDependency::write(
      expr.left.span().into(),
      RuntimeGlobals::SCRIPT_NONCE,
    )));
    return Some(true);
  }
  None
}
```

For `__webpack_require__.x` static member access in rspack mode, use the existing `member` hook in `api_plugin.rs` and add:

```rust
fn member(
  &self,
  parser: &mut JavascriptParser,
  expr: &swc_core::ecma::ast::MemberExpr,
  for_name: &str,
) -> Option<bool> {
  if parser.compiler_options.experiments.runtime_mode == RuntimeMode::Rspack
    && for_name == API_REQUIRE
    && let Some(property) = expr.prop.as_ident().map(|ident| ident.sym.as_ref())
    && let Some(runtime_global) = runtime_globals_from_property_name(property)
  {
    parser.add_error(rspack_error::error!(
      "experiments.runtimeMode: \"rspack\" does not support static __webpack_require__.{} helper access",
      property
    ).into());
    parser.add_presentational_dependency(Box::new(
      RuntimeRequirementsDependency::unsupported_require_property(
        expr.span.into(),
        runtime_global,
      ),
    ));
    return Some(true);
  }
  None
}
```

- [ ] **Step 6: Run focused parser cases**

Run:

```bash
pnpm run build:binding:dev
cd tests/rspack-test && pnpm run test -t "configCases/runtime/runtime-mode-context-setter|configCases/runtime/runtime-mode-unsupported-require-property"
```

Expected: setter case passes and unsupported static require property case errors.

- [ ] **Step 7: Commit**

```bash
git add crates/rspack_core/src/dependency/runtime_requirements_dependency.rs crates/rspack_plugin_javascript/src/parser_plugin/api_plugin.rs tests/rspack-test/configCases/runtime/runtime-mode-context-setter tests/rspack-test/configCases/runtime/runtime-mode-unsupported-require-property
git commit -m "feat: parse rspack runtime context writes"
```

---

### Task 8: Verify Require Alias Behavior

**Files:**
- Modify: `crates/rspack_plugin_javascript/src/parser_plugin/api_plugin.rs`
- Test: `tests/rspack-test/configCases/runtime/runtime-mode-require-alias`

- [ ] **Step 1: Add require alias case**

Create `tests/rspack-test/configCases/runtime/runtime-mode-require-alias/rspack.config.js`:

```js
module.exports = {
  experiments: {
    runtimeMode: "rspack"
  }
};
```

Create `tests/rspack-test/configCases/runtime/runtime-mode-require-alias/index.js`:

```js
export const requireType = typeof __webpack_require__;
export const dynamic = __webpack_require__["notAHelper"];
```

Create `tests/rspack-test/configCases/runtime/runtime-mode-require-alias/after.js`:

```js
const fs = require("fs");
const path = require("path");

const source = fs.readFileSync(path.resolve(__dirname, "dist/main.js"), "utf-8");

expect(source).toContain("\"function\"");
expect(source).toContain("__rspack_context.r[\"notAHelper\"]");
expect(source).not.toContain("__webpack_require__[\"notAHelper\"]");
```

- [ ] **Step 2: Verify dynamic root replacement uses runtime rendering**

Confirm APIPlugin still replaces the `API_REQUIRE` identifier with `RuntimeRequirementsDependency::new(..., RuntimeGlobals::REQUIRE)`. In rspack module render mode this resolves to `__rspack_context.r`, so no separate parser replacement is needed. Keep `evaluate_typeof` returning `"function"` for `API_REQUIRE`.

- [ ] **Step 3: Run focused case**

Run:

```bash
pnpm run build:binding:dev
cd tests/rspack-test && pnpm run test -t "configCases/runtime/runtime-mode-require-alias"
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/rspack_plugin_javascript/src/parser_plugin/api_plugin.rs tests/rspack-test/configCases/runtime/runtime-mode-require-alias
git commit -m "test: cover rspack require alias behavior"
```

---

### Task 9: Runtime Hook Exposure And Snapshot Isolation

**Files:**
- Test: `tests/rspack-test/configCases/runtime/runtime-mode-hook-exposure`

- [ ] **Step 1: Add hook exposure case**

Create `tests/rspack-test/configCases/runtime/runtime-mode-hook-exposure/rspack.config.js`:

```js
const { RuntimeGlobals } = require("@rspack/core");

class PublicPathRuntimeRequirementPlugin {
  apply(compiler) {
    compiler.hooks.compilation.tap("PublicPathRuntimeRequirementPlugin", compilation => {
      compilation.hooks.additionalTreeRuntimeRequirements.tap(
        "PublicPathRuntimeRequirementPlugin",
        (_chunk, set) => {
          set.add(RuntimeGlobals.publicPath);
        }
      );
    });
  }
}

module.exports = {
  experiments: {
    runtimeMode: "rspack"
  },
  plugins: [new PublicPathRuntimeRequirementPlugin()]
};
```

Create `tests/rspack-test/configCases/runtime/runtime-mode-hook-exposure/index.js`:

```js
export default 1;
```

Create `tests/rspack-test/configCases/runtime/runtime-mode-hook-exposure/after.js`:

```js
const fs = require("fs");
const path = require("path");

const source = fs.readFileSync(path.resolve(__dirname, "dist/main.js"), "utf-8");

expect(source).toContain("__rspack_context.p");
expect(source).not.toContain("__webpack_require__.p =");
```

- [ ] **Step 2: Run hook exposure case**

Run:

```bash
pnpm run build:binding:dev
cd tests/rspack-test && pnpm run test -t "configCases/runtime/runtime-mode-hook-exposure"
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tests/rspack-test/configCases/runtime/runtime-mode-hook-exposure
git commit -m "test: cover rspack runtime hook exposure"
```

---

### Task 10: Final Verification

**Files:**
- No planned source changes.

- [ ] **Step 1: Format changed Rust and JS**

Run:

```bash
pnpm run format:rs
pnpm run format:js
```

Expected: formatting completes.

- [ ] **Step 2: Build the full dev CLI**

Run:

```bash
pnpm run build:cli:dev
```

Expected: PASS.

- [ ] **Step 3: Run focused runtime-mode cases**

Run:

```bash
cd tests/rspack-test && pnpm run test -t "configCases/runtime/runtime-mode"
```

Expected: all runtime-mode cases pass.

- [ ] **Step 4: Run full unit tests**

Run:

```bash
pnpm run test:unit
```

Expected: PASS, except known watcher-only failures explicitly accepted by the user.

- [ ] **Step 5: Inspect output for forbidden legacy bridge**

Run:

```bash
rg -n "__webpack_require__\\.[a-zA-Z_$][\\w$]*\\s*=" tests/rspack-test/js/config/runtime/runtime-mode* || true
rg -n "__rspack_install_runtime|__rspack_esm_install_runtime|__webpack_require__\\.d|__webpack_require__\\.r" tests/rspack-test/js/config/runtime/runtime-mode* || true
```

Expected: no `__webpack_require__.x` bridge assignments and no install-runtime protocol in rspack runtime-mode outputs. Any remaining `__webpack_require__.x` read should be from webpack-mode snapshots or an expected-error fixture.

- [ ] **Step 6: Commit final formatting or snapshot changes**

```bash
git status --short
git add .
git commit -m "test: verify rspack runtime mode"
```

Skip the commit if `git status --short` is empty.

---

## Self-Review

- Spec coverage: the plan covers the two-value public config, `__rspack_context.r`, `MAKE_NAMESPACE_OBJECT -> ns`, module render mode, runtime module lexical render mode, runtime metadata, context setter fields, custom runtime module rejection, async chunk behavior without install-runtime, unsupported static `__webpack_require__.x`, dynamic require root replacement, hook exposure, and verification.
- Placeholder scan: no steps rely on unbounded "do the right thing"; local API entry points are named before implementation.
- Type consistency: the plan consistently uses `RuntimeMode::Webpack`, `RuntimeMode::Rspack`, `RuntimeGlobalRenderMode::{Webpack,RspackModule,RspackRuntimeModule}`, `RuntimeVariable::RuntimeContext`, `RuntimeProxyMetadata`, `context_setter_fields`, and `__rspack_context`.
