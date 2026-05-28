const fs = require("fs");
const path = require("path");

const source = fs.readFileSync(path.resolve(__dirname, "bundle0.js"), "utf-8");

expect(source).toContain("__webpack_require__[runtimeKey]");
expect(source).toContain("Object.defineProperty(__webpack_require__, item[0]");
