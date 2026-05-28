const fs = require("fs");
const path = require("path");

module.exports = {
  findBundle() {
    return ["bundle0.js"];
  },
  afterExecute(options) {
    const source = fs.readFileSync(
      path.resolve(options.output.path, "bundle0.js"),
      "utf-8"
    );

    expect(source).toContain(
      '["d", function() { return __var_d; }, function(value) { __var_d = value; }]'
    );
    expect(source).toContain(
      "Object.defineProperty(__proxy, item[0], { configurable: true, enumerable: true, get: item[1], set: item[2] })"
    );
    expect(source).toContain("__rspack_runtime.d = runtimeProxyStaticWrite");
    expect(source).not.toContain("Object.defineProperty(__webpack_require__, item[0]");
  }
};
