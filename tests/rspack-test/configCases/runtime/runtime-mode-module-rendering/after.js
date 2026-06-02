const fs = require("fs");
const path = require("path");

const source = fs.readFileSync(
  path.resolve(__dirname, "dist/main.js"),
  "utf-8",
);

expect(source).toContain("__webpack_require__.d");
expect(source).toContain("__webpack_require__.r");
expect(source).not.toContain("__rspack_context.d");
expect(source).not.toContain("__rspack_context.ns");
