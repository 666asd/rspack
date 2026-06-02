const fs = require("fs");
const path = require("path");

/** @type {import("../../../..").TConfigCaseConfig} */
module.exports = {
  afterExecute(options) {
    const source = fs.readFileSync(
      path.resolve(options.output.path, "main.js"),
      "utf-8",
    );

    expect(source).toContain("var __rspack_context = { r: __webpack_require__ };");
    expect(source).toContain("function __rspack_module_context(req)");
    expect(source).not.toContain("function __rspack_context(req)");
  },
};
