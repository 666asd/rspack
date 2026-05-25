const fs = require("fs");
const path = require("path");

const source = fs.readFileSync(path.resolve(__dirname, "bundle0.js"), "utf-8");

expect(source).toContain("changedByCustomRuntimeModule");
expect(source).toContain("__var_d");
expect(source).toContain("get: () => __var_d");
