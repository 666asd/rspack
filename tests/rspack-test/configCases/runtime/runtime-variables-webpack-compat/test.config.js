const fs = require("fs");
const path = require("path");

module.exports = {
  findBundle(i) {
    return i === 0 ? "compat.js" : "rspack-only.js";
  },
  afterExecute(options) {
    const [compatOptions, rspackOnlyOptions] = options;
    const compat = fs.readFileSync(
      path.resolve(compatOptions.output.path, "compat.js"),
      "utf-8",
    );
    const rspackOnly = fs.readFileSync(
      path.resolve(rspackOnlyOptions.output.path, "rspack-only.js"),
      "utf-8",
    );

    expect(compat).toContain("function __rspack_require");
    expect(compat).toContain("var __webpack_require__ = __rspack_require");
    expect(compat).toContain("__rspack_modules");
    expect(compat).toContain("__webpack_modules__");
    expect(compat).toContain("__rspack_module_cache");
    expect(compat).toContain("__webpack_module_cache__");

    expect(rspackOnly).toContain("function __rspack_require");
    expect(rspackOnly).toContain("__rspack_modules");
    expect(rspackOnly).toContain("__rspack_module_cache");
    expect(rspackOnly).not.toContain("__webpack_require__");
    expect(rspackOnly).not.toContain("__webpack_modules__");
    expect(rspackOnly).not.toContain("__webpack_module_cache__");
  },
};
