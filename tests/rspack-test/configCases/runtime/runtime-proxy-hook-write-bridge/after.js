const fs = require("fs");
const path = require("path");

const source = fs.readFileSync(path.resolve(__dirname, "bundle0.js"), "utf-8");

expect(source).toContain(
	'["o", function() { return __var_o; }, function(value) { __var_o = value; }]'
);
expect(source).toContain(
	"Object.defineProperty(__rspack_context, item[0], { configurable: true, enumerable: true, get: item[1], set: item[2] })"
);
expect(source).not.toContain('["o", __var_o]');
