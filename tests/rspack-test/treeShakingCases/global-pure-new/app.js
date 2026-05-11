import "./shadow";

export const marker = 1;

// === Should be tree-shaken by the side-effects heuristic ===

// Collections with no args.
let unusedSet = new Set();
let unusedMap = new Map();
let unusedWeakMap = new WeakMap();
let unusedWeakSet = new WeakSet();

// Collections with explicit nullish are also fine (return empty collection).
let unusedNullSet = new Set(null);
let unusedUndefMap = new Map(undefined);

// TypedArrays with finite non-negative numeric literal length.
let unusedTyped = new Uint8Array(16);
let unusedTypedFractional = new Uint8Array(1.5);
let unusedBuf = new ArrayBuffer(0);

// Pure type/identity checks (args themselves must be pure too).
let unusedArrIsArray = Array.isArray([1, 2, 3]);
let unusedObjectIs = Object.is(1, 2);

// String/Object with literal args — no coercion since literals don't have
// custom @@toPrimitive.
let unusedString = String("hello");
let unusedObject = Object("y");

// Boolean is pure regardless of arg shape (ToBoolean never throws).
let unusedBool = Boolean({});
let unusedBoolVar = Boolean(marker);

// Symbol() with primitive description.
let unusedSymbol = Symbol("desc");

// BigInt is safe for Number coercion.
let unusedNumberBigInt = new Number(1n);

function impureArg() { console.log("keep"); return 1; }

// === MUST be kept (may throw or may invoke user code) ===

let unusedSetLiteral = new Set(1);
let unusedMapLiteral = new Map("foo");
let unusedArrayNegative = new Array(-1);
let unusedArrayFractional = new Array(1.5);
let unusedTypedNegative = new Uint8Array(-1);
let unusedDateBigInt = new Date(1n);

// RegExp literals are pure argument expressions, but coercing them can invoke
// user code through RegExp.prototype.toString.
RegExp.prototype.toString = impureArg;
let unusedRegexToString = String(/x/);
let unusedRegexNewString = new String(/x/);
let unusedRegexSymbolDesc = Symbol(/x/);

let dynamic = { length: 16 };
let unusedWithDynamic = new Uint8Array(dynamic);

// Impure nested arguments are still kept.

let unusedWithImpureArg = new Set([impureArg()]);
let unusedBoolImpure = Boolean(impureArg());
