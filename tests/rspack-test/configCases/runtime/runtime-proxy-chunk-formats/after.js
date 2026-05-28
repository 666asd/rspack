const fs = require("fs");
const path = require("path");

for (const file of ["web.js", "commonjs.js", "module.js"]) {
	const source = fs.readFileSync(path.resolve(__dirname, "dist", file), "utf-8");
	expect(source).toContain("__rspack_runtime");
	expect(source).toContain("__var_");
}

for (const file of ["728.web.js", "728.commonjs.js", "728.module.js"]) {
	const source = fs.readFileSync(path.resolve(__dirname, "dist", file), "utf-8");
	expect(source).toContain("__rspack_runtime");
	expect(source).not.toContain("__var_");
	expect((source.match(/\b(?:let|var) __rspack_runtime\b/g) || []).length).toBe(1);
}

const commonjsAsync = fs.readFileSync(
	path.resolve(__dirname, "dist", "728.commonjs.js"),
	"utf-8"
);
expect(commonjsAsync).toContain("exports.__rspack_install_runtime");
expect(commonjsAsync).not.toContain("exports.__rspack_runtime");
const commonjsRuntime = fs.readFileSync(
	path.resolve(__dirname, "dist", "commonjs.js"),
	"utf-8"
);
expect(commonjsRuntime).toContain(
  'typeof chunk.__rspack_install_runtime === "function"'
);
expect(commonjsRuntime).not.toContain("chunk.__rspack_runtime");

const moduleAsync = fs.readFileSync(
	path.resolve(__dirname, "dist", "728.module.js"),
	"utf-8"
);
expect(moduleAsync).toContain("__rspack_esm_install_runtime");
expect(moduleAsync).toContain("export const __rspack_esm_install_runtime");
expect(moduleAsync).not.toContain("export let __rspack_esm_install_runtime");
expect(moduleAsync).not.toContain("__rspack_install_runtime__");
expect(moduleAsync).not.toContain("__rs_erp");
const moduleRuntime = fs.readFileSync(
	path.resolve(__dirname, "dist", "module.js"),
	"utf-8"
);
expect(moduleRuntime).toContain("data.__rspack_esm_install_runtime");
expect(moduleRuntime).toContain("__rspack_esm_runtime");
expect(moduleRuntime).not.toContain("__rs_er");

const webRuntime = fs.readFileSync(
	path.resolve(__dirname, "dist", "web.js"),
	"utf-8"
);
expect(webRuntime).toContain(
  "chunkIds, moreModules, runtime, __rspack_install_runtime"
);
expect(webRuntime).toContain('typeof __rspack_install_runtime === "function"');
expect(webRuntime).not.toContain("data.length > 3");
