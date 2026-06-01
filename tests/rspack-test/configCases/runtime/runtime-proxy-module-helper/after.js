const fs = require("fs");
const path = require("path");

const source = fs.readFileSync(path.resolve(__dirname, "bundle0.js"), "utf-8");

expect(source).toContain("__rspack_context.d");
expect(source).not.toContain("__webpack_require__.d(__webpack_exports__");
expect(source).toContain("let __var_d");
expect(source).not.toContain("__webpack_require__.d = function");
expect(source).toContain("let __rspack_context");
expect(source).toContain('["d", __var_d]');
expect(source).toContain(
	'if (typeof item[1] !== "undefined") __rspack_context[item[0]] = item[1]'
);
expect(source).toContain("(() => { let __fields");
expect(source).toContain("for (let i = 0; i < __fields.length; i++)");
expect(source).not.toContain(".forEach(");
expect(source).not.toContain("Object.defineProperty(__rspack_context, \"d\"");
expect(source).not.toContain("__webpack_require__.d = __var_d");
