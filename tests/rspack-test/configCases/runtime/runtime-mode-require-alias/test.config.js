const fs = require("fs");
const path = require("path");

/** @type {import("../../../..").TConfigCaseConfig} */
module.exports = {
  findBundle() {
    return ["./main.js"];
  },
  afterExecute(options) {
    const source = fs.readFileSync(
      path.resolve(options.output.path, "main.js"),
      "utf-8",
    );

    expect(source).toContain('"function"');
    expect(source).toContain("__rspack_context.r(");
    expect(source).toContain("exports.value = 42");
    expect(source).toContain('__rspack_context.r["notAHelper"]');
    expect(source).not.toContain('__webpack_require__("./lib")');
    expect(source).not.toContain('__webpack_require__["notAHelper"]');
  },
};
