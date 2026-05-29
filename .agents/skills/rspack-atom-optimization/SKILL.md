---
name: rspack-atom-optimization
description: Use when auditing or optimizing Rspack Rust code that uses swc_atoms::Atom, swc_core::atoms::Atom, or rspack_util::atom::Atom in hot Eq, Hash, HashMap/HashSet, parser, export-name, generated-name, or cross-thread paths. Trigger when Atom appears in profiles, when repeated Atom::from/Atom::new conversions happen in loops, when Atom keys are used in maps/sets, or when Atom values are recreated across threads and later compared or used as keys.
---

# Rspack Atom Optimization

Optimize `Atom` only when the access pattern benefits from it. Keep changes small, local, and justified by a hot path, repeated loop, high-cardinality container, or profile evidence.

## Mental Model

- `rspack_util::atom::Atom` is `swc_core::atoms::Atom`, backed by `swc_atoms`/`hstr`.
- Default 64-bit `hstr::Atom` is one word; `Option<Atom>` is also one word.
- Small strings are inline in the tagged word. On the default 64-bit build this is usually up to 7 bytes.
- Dynamic strings store bytes in `ThinArc` entries with a precomputed `rustc_hash::FxHasher` hash.
- `Atom::hash` writes one `u64`; hashing an existing atom avoids scanning string bytes.
- `Atom::eq` first compares the raw tagged value. Matching raw values are the fastest path.
- If two dynamic atoms have different backing entries, equality compares precomputed hash first, then string bytes.
- `Atom::from(...)` uses a thread-local global `AtomStore`.
- Dynamic strings up to 512 bytes are interned only inside that one `AtomStore`; strings over 512 bytes are not deduplicated even inside the same store.
- Recreating the same dynamic text in different threads can produce multiple backing entries. The atoms still compare equal, but they lose the raw-handle equality fast path.
- Moving or cloning an existing `Atom` handle across threads preserves the same backing entry and the fast path.
- `Ustr` is process-global and pointer-sized. Recreating the same text in different threads returns the same pointer, and `UstrMap`/`UstrSet` use `ustr::IdentityHasher`.

Local microbenchmarks showed this practical shape for repeated equality over the same dynamic text: `Ustr` pointer equality was fastest; `Atom` values created in the same thread were close behind; `Atom` values independently recreated on another thread were roughly an order of magnitude slower and close to comparing equal `String` contents. For repeated map lookup, `UstrMap`/`UstrSet` with `ustr::IdentityHasher` were much faster than default `String` keys because they reuse the stored hash and pointer identity. Treat these as directional results: preserve `Atom` provenance across threads, and prefer `Ustr`/`Identifier` when the same text must be recreated independently across workers.

## Audit

1. Confirm the code is hot enough to matter. Prefer profile evidence, large repeated loops, high-cardinality containers, or cross-thread aggregation.
2. Search for Atom creation and Atom-keyed containers:

```sh
rg -n 'Atom::from|Atom::new|atom!\(|lazy_atom!|FxHash(Map|Set)<Atom|FxIndex(Map|Set)<Atom|Hash(Map|Set)<Atom|Vec<Atom>|&Atom' crates -g '*.rs'
```

3. Classify each Atom's provenance:

- **AST/parser-local**: comes from SWC AST identifiers/literals and stays in the same parser or transform flow.
- **Static literal**: known compile-time text such as `"default"`, `"__esModule"`, runtime helper names, or protocol names.
- **Generated candidate**: created with `format!`, `to_string`, escaping, suffix loops, or render-time symbol generation.
- **Cross-thread recreated**: independently converted from the same `&str`/`String` in multiple workers, then compared or used as keys later.
- **Rspack identifier-like**: module id, runtime name, chunk id, request, resource identifier, or project-wide key that already has an Rspack key type.

4. Fix the smallest bad pattern. Do not rewrite public APIs for a micro-optimization unless an end-to-end benchmark or profile supports it.

## Preferred Fixes

- Reuse existing AST atoms. Prefer cloning/passing the original `Atom` over `Atom::from(atom.as_str())`, `Atom::from(name.clone())`, or `Atom::new(name)`.
- For cross-thread work, create atoms once and move/clone the handles into workers. If workers must recreate the same process-wide text independently, consider `Ustr`/`Identifier` instead of `Atom`.
- For Rspack-wide keys, prefer existing `Identifier`, `IdentifierMap`, `IdentifierSet`, `Ustr`, `UstrMap`, or `UstrSet`.
- For generated-name collision loops, keep a `String` scratch value until the candidate is selected when possible. If membership checks dominate, maintain a parallel cheaper key set rather than interning every failed candidate.
- For temporary lookup keys, avoid allocating a fresh `Atom` only to call `contains`/`get`; preserve the original atom, change the key type, or add a borrowed/string side index.
- For static literals, prefer `atom!("literal")`. Inline literals are already cheap, but long literals through `atom!` use an internal static cache.
- Use `LazyLock<Atom>` only when callers can borrow `&Atom` and profiling shows clone/drop cost matters. Do not add `thread_local!` just to cache an Atom literal.

## Hasher Guidance

- `Atom::hash` already avoids string-byte hashing, but the caller's hasher still mixes the emitted `u64`.
- Do not blindly use an identity/no-op hasher for `Atom` maps. Inline atoms emit a tagged raw value rather than a fully mixed content hash, so a no-op hasher can have poor distribution for many short keys.
- If an Atom-keyed map is still hot, measure before changing the hasher. Keep a no-op hasher local to a proven dynamic-key-only workload, and check collision behavior.
- Prefer `UstrMap`/`UstrSet` when identity hashing is desired; `Ustr` stores a real precomputed hash and the crate provides `ustr::IdentityHasher` for that exact use.

## Keep Atom When

- The value already comes from SWC AST and must interoperate with SWC AST APIs.
- The value is a JS export/member/property name and stays in parser/transform/export-name code.
- Same-provenance atoms are passed or cloned through the workflow.
- The string is small, parser-local, and used as a compact symbol rather than a process-wide key.

## Avoid

- Recreating dynamic atoms from the same text independently in rayon workers and deduplicating them later.
- Using `Atom` for module identifiers, chunk ids, runtime ids, requests, or resource identifiers when Rspack already has a key type.
- Interning unbounded unique generated text into a global/process-wide cache without checking memory lifetime.
- Replacing SWC AST atom fields with `Ustr`.
- Adding parallelism to hide Atom overhead instead of fixing the representation or conversion pattern.

## Validation

Use the narrowest checks that cover the touched code:

```sh
cargo fmt -p <crate> --check
cargo check -p <crate>
```

Add or update behavior tests when changes affect deduplication, export-name ordering, generated-name collisions, or cross-thread behavior.

For performance claims, prefer a targeted Rspack fixture plus a real benchmark/profile from the affected feature. Include at least one case that exercises the original bad pattern, such as cross-thread recreated atoms, generated-name collision loops, or Atom-keyed lookup from borrowed text.
