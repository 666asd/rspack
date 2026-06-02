# Runtime Proxy Context Design

## Summary

This design reworks Rspack's experimental runtime proxy plan around a module execution context. In `experiments.runtimeMode: "rspack"`, runtime modules initialize their helpers as lexical variables in the runtime chunk, then expose only the module-visible capabilities on `__rspack_context`. Normal modules receive `__rspack_context` as their module factory third argument instead of receiving `__webpack_require__`.

The core split is:

- `__webpack_require__` remains the real module loader inside the bootstrap.
- `__rspack_context.r` is the module-visible load API and points to `__webpack_require__`.
- Runtime helper reads in normal modules render as `__rspack_context.<runtime key>`, for example `__rspack_context.d`.
- Runtime helper implementation code inside runtime modules renders as lexical variables, for example `definePropertyGetters`.

The feature is behind a small public config surface:

```js
module.exports = {
  experiments: {
    runtimeMode: "webpack" // or "rspack"
  }
};
```

`"webpack"` is the default and preserves current output. `"rspack"` enables the context runtime model. No additional compatibility or warning modes are planned for this design.

## Goals

- Decouple runtime module implementation variables from `__webpack_require__.x`.
- Keep ordinary module factory execution simple by passing one context object as the existing third argument.
- Preserve the existing module loader internally as `__webpack_require__`.
- Avoid async chunk special handling by ensuring all module factories are executed by the same loader path with the same context argument.
- Keep RuntimeGlobals collection and runtime module injection mostly intact.
- Reject unsupported custom runtime module code in `runtimeMode: "rspack"`.
- Keep webpack mode output unchanged.

## Non-Goals

- Do not implement any `runtimeMode` values beyond `"webpack"` and `"rspack"`.
- Do not add a bridge from runtime lexical variables back to `__webpack_require__.x`.
- Do not support custom runtime modules in `runtimeMode: "rspack"`.
- Do not make async chunks install or import a separate runtime proxy object.
- Do not change module factory arity. The third parameter is reused as `__rspack_context`.

## Public Configuration

`experiments.runtimeMode` accepts two values:

- `"webpack"`: current behavior. Runtime helpers render to and are attached to `__webpack_require__.x`.
- `"rspack"`: new behavior. Normal modules read runtime helpers from `__rspack_context`, while runtime modules use lexical variables internally.

The default is `"webpack"`.

The TypeScript, normalization, defaults, adapter, NAPI raw options, core options, and Rust builder surfaces should all use this two-value model.

## Runtime Context Shape

In `runtimeMode: "rspack"`, bootstrap creates a context object in the runtime chunk:

```js
var __rspack_context = {
  r: __webpack_require__
};
```

`__rspack_context.r` is the only module-visible alias for the real module loader. Runtime helper fields mostly use the existing webpack RuntimeGlobals property keys, with one intentional exception: `MAKE_NAMESPACE_OBJECT` moves away from `r` so that `r` can mean require.

```js
__rspack_context.d = definePropertyGetters;
__rspack_context.ns = makeNamespaceObject;
__rspack_context.nc = scriptNonce;
```

The `r` property is reserved for module loading. RuntimeGlobals helper keys continue to use their existing short keys except where they conflict with reserved context fields. In this phase, `MAKE_NAMESPACE_OBJECT` renders as `ns` on `__rspack_context`, while webpack mode keeps the legacy `__webpack_require__.r`.

## Module Factory Execution

All module factory call sites must pass `__rspack_context` as the third argument in `runtimeMode: "rspack"`:

```js
__webpack_modules__[moduleId](module, module.exports, __rspack_context);
```

This includes:

- normal require execution,
- `THIS_AS_EXPORTS` execution,
- `INTERCEPT_MODULE_EXECUTION`,
- entry startup paths that directly invoke factories,
- any chunk format path that directly calls a module factory.

Async chunks do not need special runtime proxy installation. They only contribute module factories to the shared module table. When an async module is executed later, the runtime loader still passes the same `__rspack_context` third argument.

## RuntimeGlobals Rendering

Rendering has three semantic modes.

### Webpack Mode

This is current behavior:

```js
RuntimeGlobals.REQUIRE -> __webpack_require__
RuntimeGlobals.DEFINE_PROPERTY_GETTERS -> __webpack_require__.d
RuntimeGlobals.MAKE_NAMESPACE_OBJECT -> __webpack_require__.r
```

### Rspack Module Mode

Used when rendering ordinary module code:

```js
RuntimeGlobals.REQUIRE -> __rspack_context.r
RuntimeGlobals.DEFINE_PROPERTY_GETTERS -> __rspack_context.d
RuntimeGlobals.MAKE_NAMESPACE_OBJECT -> __rspack_context.ns
RuntimeGlobals.SCRIPT_NONCE -> __rspack_context.nc
```

Only concrete require-scope RuntimeGlobals render through context. Sentinel or broad flags such as `REQUIRE_SCOPE` do not render as a context property.
The context key renderer starts from the legacy RuntimeGlobals property key, then applies reserved-key overrides such as `MAKE_NAMESPACE_OBJECT: "r" -> "ns"`.

### Rspack Runtime Module Mode

Used when rendering runtime module implementation code:

```js
RuntimeGlobals.DEFINE_PROPERTY_GETTERS -> definePropertyGetters
RuntimeGlobals.MAKE_NAMESPACE_OBJECT -> makeNamespaceObject
RuntimeGlobals.SCRIPT_NONCE -> scriptNonce
```

Lexical names are camelCase names derived from RuntimeGlobals names, without a prefix. They are declared inside the runtime chunk/runtime module scope. Minification can shorten these names later, so the source form optimizes for readability and deterministic snapshots.

`RuntimeGlobals.REQUIRE` is not runtime module metadata. Runtime modules may still render the real loader as `__webpack_require__` when they need bootstrap-level loading behavior.

## Runtime Proxy Metadata

Add a runtime chunk level metadata artifact for `runtimeMode: "rspack"`. It does not replace existing runtime requirements. It records only data needed to render `__rspack_context`.

Fields:

- `module_proxy_requirements`: RuntimeGlobals used by ordinary modules and exposed on `__rspack_context`.
- `runtime_module_requirements`: RuntimeGlobals used inside runtime modules and declared as lexical variables.
- `context_setter_fields`: RuntimeGlobals whose context fields need setter synchronization because parser/codegen found a supported write.
- `hook_exposed_requirements`: RuntimeGlobals added by `additionalTreeRuntimeRequirements` and exposed for reading.
- `has_custom_runtime_module`: whether the runtime chunk contains custom runtime source.

The metadata is computed at the end of `process_chunks_runtime_requirements`, after tree runtime requirements are merged and runtime modules have been added to the chunk graph.

The collection flow is:

1. Iterate each entry/runtime chunk and its referenced downstream chunks.
2. For ordinary modules, collect renderable require-scope RuntimeGlobals into `module_proxy_requirements`.
3. For runtime modules, collect renderable require-scope additional runtime requirements into `runtime_module_requirements`.
4. Record custom runtime modules and fail the compilation in `runtimeMode: "rspack"`.
5. Read write metadata from code generation results and add it to `context_setter_fields`.
6. Read `additionalTreeRuntimeRequirements` contributions and expose their renderable require-scope RuntimeGlobals as read-only context fields.

`RuntimeGlobals.REQUIRE` is excluded from metadata. `__rspack_context.r` is a bootstrap field.

## Context Field Rendering

Runtime modules initialize lexical variables first:

```js
var definePropertyGetters;
var makeNamespaceObject;
var scriptNonce;

definePropertyGetters = function(exports, definition) {};
makeNamespaceObject = function(exports) {};
scriptNonce = "";
```

Fields without setter requirements are assigned directly:

```js
__rspack_context.d = definePropertyGetters;
__rspack_context.ns = makeNamespaceObject;
```

Fields with setter requirements use `Object.defineProperty` so module writes update the lexical variable:

```js
Object.defineProperty(__rspack_context, "nc", {
  configurable: true,
  enumerable: true,
  get: function() {
    return scriptNonce;
  },
  set: function(value) {
    scriptNonce = value;
  }
});
```

When multiple setter fields are needed, generate a compact loop rather than repeating large `Object.defineProperty` blocks. The loop should honor `output.environment`: use `let` and arrow functions only when the target supports them; otherwise use `var` and function expressions.

## Parser And Dependency Behavior

Supported runtime API writes, such as:

```js
__webpack_nonce__ = "nonce";
```

are transformed through `RuntimeRequirementsDependency::Write`. The dependency records the RuntimeGlobal that needs a setter-capable context field. The generated module code writes to `__rspack_context.nc`.

`__webpack_require__` is treated as a module load function alias in `runtimeMode: "rspack"`:

- `__webpack_require__(request)` renders as `__rspack_context.r(request)`.
- dynamic member roots such as `__webpack_require__[expr]` render with the root replaced by `__rspack_context.r`.
- `typeof __webpack_require__` remains `"function"`.

Static helper property access on `__webpack_require__` is not supported:

```js
__webpack_require__.d;
__webpack_require__.d = value;
```

These should produce compile-time errors in `runtimeMode: "rspack"` because helper properties are no longer exposed on `__webpack_require__`.

Webpack mode keeps all current parser behavior.

## Custom Runtime Modules

Custom runtime modules are unsupported in `runtimeMode: "rspack"` and should produce a clear compilation error. This includes runtime modules with custom source and JS API runtime modules such as `RuntimeModuleFromJs`.

The error should explain that rspack runtime mode does not expose `__webpack_require__.x` and custom runtime module code cannot be safely rewritten.

## Async Chunks

Async chunks need no special context installation. They should continue to register module factories and runtime modules through the existing chunk format mechanisms.

The important invariant is that module factories are not executed by async chunks directly. They are installed into the module table, and later executed by the runtime loader:

```js
function __webpack_require__(moduleId) {
  var module = { exports: {} };
  __webpack_modules__[moduleId](module, module.exports, __rspack_context);
  return module.exports;
}
```

Therefore an async module receives the same context as an initial module:

```js
// async chunk factory
__webpack_modules__["./lazy.js"] = function(module, exports, __rspack_context) {
  __rspack_context.d(exports, {
    value: function() {
      return value;
    }
  });
};
```

Chunk loading runtime modules may still use lexical runtime variables internally. They do not need an exported `installRuntime` helper for ordinary module execution.

## Error Handling

Errors should be direct and narrow:

- `runtimeMode: "rspack"` plus custom runtime module: compilation error.
- static `__webpack_require__.x` read/write in user module: compilation error.
- unsupported composite RuntimeGlobals rendering in context or lexical mode: internal panic or explicit error during development, not silent invalid output.

Dynamic `__webpack_require__[expr]` is intentionally not rejected in this phase. It only gets the root replacement behavior and receives no runtime helper compatibility guarantee.

## Testing Plan

Add focused config cases for:

- webpack mode disabled path: no `__rspack_context`.
- basic ESM helper output: module code uses `__rspack_context.d` and `__rspack_context.ns`; runtime modules use lexical camelCase names.
- require API behavior: calls become `__rspack_context.r(...)`; dynamic roots become `__rspack_context.r[...]`; `typeof __webpack_require__` remains `"function"`.
- static `__webpack_require__.d` read/write errors.
- Runtime API setter behavior, for example `__webpack_nonce__ = "x"` creates a setter field on `__rspack_context.nc`.
- read-only context exposure for RuntimeGlobals added through `additionalTreeRuntimeRequirements`.
- custom runtime module errors.
- async chunk module factories receiving `__rspack_context` without async runtime proxy installation.

Then add a dedicated rspack runtime test entry that enables `runtimeMode: "rspack"` for a controlled config/hot subset. Cases whose output matches webpack mode can reuse existing snapshots. Cases with changed output should use isolated runtime-mode snapshots. Cases that rely on custom runtime modules or static `__webpack_require__.x` helper access should be expected errors or explicit skips for this phase.

Final local verification before implementation completion should include:

- `pnpm run build:cli:dev`
- focused runtime proxy config cases
- the dedicated rspack runtime test entry
- `pnpm run test:unit` when practical, with baseline failures separated from runtime-mode regressions

## Implementation Boundaries

Keep the implementation scoped to:

- config plumbing,
- runtime global rendering helpers and template modes,
- module factory wrapper/call argument rendering,
- runtime proxy metadata collection,
- runtime module lexical declaration/context exposure rendering,
- parser/dependency write/error handling,
- focused tests.

Do not refactor unrelated runtime modules, chunk graph algorithms, or HMR behavior unless a directly affected test proves it is necessary.

## Open Follow-Ups

- Whether context helper keys should remain short forever or later support readable dev names.
- Whether custom runtime modules can opt into a context-aware API in a later phase.
