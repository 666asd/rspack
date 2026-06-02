const fs = require("fs");
const path = require("path");

/** @type {import("../../../..").TConfigCaseConfig} */
module.exports = {
  findBundle() {
    return ["main.js"];
  },
  afterExecute(options) {
    const source = fs.readFileSync(
      path.resolve(options.output.path, "main.js"),
      "utf-8",
    );

    expect(source).toContain("__webpack_require__.d");
    expect(source).toContain("__webpack_require__.r");
    expect(source).not.toContain("__rspack_context.d");
    expect(source).not.toContain("__rspack_context.ns");
  },
};
