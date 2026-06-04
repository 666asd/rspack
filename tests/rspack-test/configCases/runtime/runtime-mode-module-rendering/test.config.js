const fs = require("fs");
const path = require("path");

/** @type {import("../../../..").TConfigCaseConfig} */
module.exports = {
  afterExecute(options) {
    const source = fs.readFileSync(
      path.resolve(options.output.path, "main.js"),
      "utf-8",
    );

    expect(source).toContain(
      'var __rspack_context = typeof __rspack_context !== "undefined" ? __rspack_context : {};',
    );
    expect(source).toContain("__rspack_context.d");
    expect(source).toContain("__rspack_context.N");
    expect(source).toContain('Object.defineProperty(__rspack_context, "d"');
    expect(source).toContain('Object.defineProperty(__rspack_context, "N"');
    expect(source).toContain("module.exports, __rspack_context");
    expect(source).toContain("definePropertyGetters =");
    expect(source).toContain("makeNamespaceObject =");
    expect(source).not.toContain("__webpack_require__.d(__webpack_exports__");
    expect(source).not.toContain("__webpack_require__.r(__webpack_exports__");
    expect(source).not.toContain("__webpack_require__.d =");
    expect(source).not.toContain("__webpack_require__.r =");
  },
};
