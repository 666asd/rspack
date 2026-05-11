// This module is imported only for side effects. With builtinPureGlobals enabled,
// all top-level statements below should be considered side-effect-free and the
// whole module should be dropped.
new Set();
new Map(undefined);
new Uint8Array(16);
Array.isArray([1, 2, 3]);
Object.is(1, 2);
String("hello");
Object("y");
Boolean({});
Boolean(1);
Symbol("desc");
new Number(1n);
