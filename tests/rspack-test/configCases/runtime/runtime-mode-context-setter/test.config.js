const fs = require("fs");
const path = require("path");

/** @type {import("../../../..").TConfigCaseConfig} */
module.exports = {
  afterExecute(options) {
    const source = fs.readFileSync(
      path.resolve(options.output.path, "main.js"),
      "utf-8",
    );

    expect(source).toContain("__rspack_context.nc");
    expect(source).toContain('Object.defineProperty(__rspack_context, "nc"');
    expect(source).toContain("set: function(value)");
    expect(source).toContain("scriptNonce = value");
    expect(source).not.toContain("__webpack_require__.nc");
  },
};
