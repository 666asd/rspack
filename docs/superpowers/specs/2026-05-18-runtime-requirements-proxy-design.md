# Runtime Requirements Proxy Design

## Summary

This design decouples runtime requirement collection from runtime global reference rendering. Module code generation still records `RuntimeGlobals` in `CodeGenerationResult.runtime_requirements`, so the existing module, chunk, and tree runtime requirement passes remain the source of truth. The rendering of runtime global references becomes context-sensitive:

- Module bodies render `REQUIRE_SCOPE_GLOBALS` through a runtime proxy object, for example `__rspack_runtime_proxy__.d`.
- Runtime modules render `REQUIRE_SCOPE_GLOBALS` as lexical variables in the runtime chunk wrapper scope.
- `RuntimeGlobals::REQUIRE` continues to render as `__webpack_require__`.
- Module factory signatures remain unchanged.

The first implementation is behind an experimental flag. When the flag is disabled, output should preserve the current behavior.

## Goals

- Preserve webpack plugin compatibility as the primary boundary.
- Keep module factory parameters unchanged, especially the third `__webpack_require__` argument.
- Move internal runtime module references away from `__webpack_require__.x` and into runtime chunk lexical variables.
- Expose runtime capabilities to module bodies only through `__rspack_runtime_proxy__`.
- Keep the existing `render_runtime_globals` API shape for the first phase.
- Migrate all `REQUIRE_SCOPE_GLOBALS` in the experimental path.
- Provide a one-way compatibility bridge from lexical runtime variables back to `__webpack_require__.x`.

## Non-goals

- Do not implement per-module proxy shapes.
- Do not rewrite generated sources with string replacement.
- Do not require JavaScript or custom runtime modules to migrate to a new API in the first phase.
- Do not support third-party runtime modules reassigning `__webpack_require__.x` and expecting internal lexical variables to update.
- Do not change module factory signatures or add a fourth module factory argument.

## Current Coupling

Today `ModuleCodeTemplate::render_runtime_globals(...)` does two things at once:

1. It records the requested `RuntimeGlobals` into the module code generation result.
2. It renders the JavaScript reference, often as `__webpack_require__.x`.

`runtime_requirements_pass` later merges module runtime requirements into chunk and tree requirements, runs the runtime requirement hooks, and adds runtime modules. Runtime modules are generated later and also commonly render runtime globals as `__webpack_require__.x`. This makes runtime modules visible as properties on the require function, which weakens dead code elimination because modules can access broad runtime state through `__webpack_require__`.

## Proposed Architecture

### Runtime Global Render Modes

Add a render strategy layer used by `ModuleCodeTemplate` and `RuntimeCodeTemplate`:

- `RequireProperty`: current behavior. `RuntimeGlobals::DEFINE_PROPERTY_GETTERS` renders as `__webpack_require__.d`.
- `ModuleProxy`: module body behavior. `DEFINE_PROPERTY_GETTERS` renders as `__rspack_runtime_proxy__.d`.
- `LexicalRuntime`: runtime module behavior. `DEFINE_PROPERTY_GETTERS` renders as a stable, readable lexical variable such as `__rspack_runtime_define_property_getters__`.

`render_runtime_globals(...)` continues to collect requirements when called on `ModuleCodeTemplate`. `render_runtime_globals_without_adding(...)` only renders, but it uses the same mode.

Only `REQUIRE_SCOPE_GLOBALS` participate in proxy or lexical rendering. `REQUIRE`, `EXPORTS`, `MODULE`, and `MODULE_GLOBALS` keep their existing semantics.

### Centralized Mapping

Keep `runtime_globals_to_string(...)` as the legacy renderer. Add centralized helpers:

- `runtime_globals_to_proxy_property(...)`: returns the proxy property key, matching the existing short `__webpack_require__.x` property key.
- `runtime_globals_to_lexical_variable(...)`: returns a stable, readable variable name used inside the runtime chunk wrapper.
- `RuntimeVariable::RuntimeProxy`: renders as `__rspack_runtime_proxy__`.

Composite or unsupported runtime global bitsets should be rejected or explicitly routed through legacy rendering. They must not silently produce invalid proxy or lexical references.

## Runtime Chunk Boundary

Runtime chunk scope is the capability boundary. Runtime modules and `__rspack_runtime_proxy__` are emitted inside the runtime chunk wrapper. Runtime modules can refer to each other by lexical variables. Module factories capture `__rspack_runtime_proxy__` from the outer runtime chunk scope.

The module factory third parameter remains `__webpack_require__`; only runtime helper references in module bodies change from `__webpack_require__.x` to `__rspack_runtime_proxy__.x`.

## Runtime Proxy Metadata

During or after `runtime_requirements_pass`, produce runtime chunk level metadata:

- `module_proxy_requirements`: the union of `REQUIRE_SCOPE_GLOBALS` used by ordinary modules that execute under the runtime chunk.
- `runtime_module_requirements`: `REQUIRE_SCOPE_GLOBALS` required by runtime modules in the runtime chunk.
- `has_custom_runtime_module`: true when a JavaScript/custom runtime module or other non-statically-analyzable runtime module is present.
- `proxy_fields`: the final fields exposed through `__rspack_runtime_proxy__` and bridged back to `__webpack_require__`.

Field selection:

- Without custom runtime modules, `proxy_fields` is derived from `module_proxy_requirements`. This keeps the module-visible surface as small as possible.
- With custom runtime modules, `proxy_fields` expands to the runtime chunk's runtime-module-related `REQUIRE_SCOPE_GLOBALS`, because custom source may read old `__webpack_require__.x` properties.

The metadata affects runtime module source and chunk output, so it must participate in runtime module and chunk hashing.

## Runtime Module Generation

In the experimental path, runtime module code generation uses `LexicalRuntime` mode:

- `RuntimeGlobals::REQUIRE` still renders as `__webpack_require__`.
- `REQUIRE_SCOPE_GLOBALS` render as stable lexical variable names.
- Runtime module sources assign to and read from lexical variables instead of `__webpack_require__.x`.

Lexical declarations are emitted in the runtime chunk wrapper as `var name;`. Runtime modules remain responsible for initializing the variable or the object/function it represents. This is important for globals that were previously initialized as objects, arrays, or handler maps, such as ensure chunk handlers, share scopes, and HMR handler maps.

## Runtime Chunk Rendering

When rendering the runtime chunk under the experimental path without custom runtime modules, emit in this order:

1. Lexical declarations for runtime globals used by runtime modules and bridge fields.
2. Runtime module sources, rendered in `LexicalRuntime` mode.
3. Runtime proxy creation using the selected `proxy_fields`.
4. One-way compatibility bridge assignments from lexical variables to `__webpack_require__.x`.
5. Existing startup/module execution code.

Example shape:

```js
var __rspack_runtime_define_property_getters__;
var __rspack_runtime_create_fake_namespace_object__;

// runtime module sources assign lexical variables
__rspack_runtime_define_property_getters__ = function(exports, definition) {};
__rspack_runtime_create_fake_namespace_object__ = function(value, mode) {};

var __rspack_runtime_proxy__ = {
  d: __rspack_runtime_define_property_getters__,
  t: __rspack_runtime_create_fake_namespace_object__
};

__webpack_require__.d = __rspack_runtime_define_property_getters__;
__webpack_require__.t = __rspack_runtime_create_fake_namespace_object__;
```

The bridge is one-way. If a custom runtime module later reassigns `__webpack_require__.d`, internal lexical references are not updated. Users who rely on that mutation behavior need the legacy path.

When custom runtime modules are present, bridge reads must be available while runtime modules execute. In that case, initialize the legacy surface before runtime module sources with getter-based or equivalent one-way aliases from `__webpack_require__.x` to the current lexical variable value. This preserves reads from custom runtime modules without allowing writes to update lexical variables.

## Data Flow

1. `code_generation_pass`
   - Module code generation records runtime requirements as before.
   - In the experimental path, module references to `REQUIRE_SCOPE_GLOBALS` render through `__rspack_runtime_proxy__.x`.

2. `runtime_requirements_pass`
   - Existing module, chunk, and tree requirement merging remains.
   - Runtime modules are still added by existing hooks.
   - Runtime proxy metadata is calculated per runtime chunk.

3. `create_hash_pass`
   - Runtime module code generation renders runtime globals in lexical mode.
   - Hashing includes the render mode and proxy metadata that affect output.

4. `create_chunk_assets_pass`
   - Runtime chunks render lexical declarations, runtime module sources, runtime proxy, and one-way bridge.
   - When custom runtime modules are present, legacy bridge reads are installed before runtime module sources execute.
   - Module factories keep the current signature and capture `__rspack_runtime_proxy__` from the runtime chunk scope.

## Compatibility

The experimental path prioritizes webpack plugin compatibility:

- Existing custom runtime modules can still read `__webpack_require__.x` because bridge assignments are emitted.
- Existing module factory call conventions are preserved.
- Runtime proxy field names match the old short require property names.

The first phase does not support reverse synchronization from custom writes to `__webpack_require__.x` back into lexical runtime variables.

## Risks

- Some runtime modules currently rely on bootstrap code or previous runtime modules to initialize objects like `__webpack_require__.f`. After lexical rendering, the corresponding runtime module must explicitly initialize the lexical variable before it is bridged.
- Different chunk formats have different wrapper shapes. Array push, CommonJS, module, and standard JavaScript runtime chunks must all preserve the closure relationship between module factories and `__rspack_runtime_proxy__`.
- Incremental compilation and caches may reuse old runtime module sources unless render mode and proxy metadata are part of invalidation or hashing.
- HMR and module federation use broad runtime surfaces and custom runtime code. They need focused compatibility tests.

## Testing Plan

- Snapshot output for module bodies to verify `__rspack_runtime_proxy__.x` replaces `__webpack_require__.x` for `REQUIRE_SCOPE_GLOBALS`.
- Snapshot output for runtime modules to verify lexical variables replace `__webpack_require__.x`.
- Add a case with no custom runtime module and assert proxy/bridge fields only include module-collected requirements.
- Add a case with a custom runtime module and assert proxy/bridge fields expand to the runtime chunk's runtime module requirements.
- Add a documented limitation case showing reassignment of `__webpack_require__.x` does not affect lexical internal references.
- Cover ESM helpers, dynamic import/chunk loading, HMR, module federation, and multiple chunk formats.

## Rollout

Implement behind an experimental flag. Keep the legacy path as the default until output, cache behavior, and compatibility tests are validated. The flag can later be promoted once the bridge and runtime proxy metadata have proven stable.
