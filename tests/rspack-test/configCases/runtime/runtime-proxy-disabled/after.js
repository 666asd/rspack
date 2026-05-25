const fs = require("fs");
const path = require("path");

const source = fs.readFileSync(path.resolve(__dirname, "bundle0.js"), "utf-8");

expect(source).not.toContain("__rspack_runtime.");
expect(source).not.toContain("let __rspack_runtime");
