// This module is also imported only for side effects, but these built-ins may
// throw or invoke user code and must keep the module alive.
function impureArg() { console.log("keep side effects"); return 1; }

new Set(1);
new Array(-1);
new Uint8Array(-1);
String(/x/);
new Set([impureArg()]);
