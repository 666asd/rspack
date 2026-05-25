const fs = require("fs");
const path = require("path");

const source = fs.readFileSync(path.resolve(__dirname, "bundle0.js"), "utf-8");

expect(source).toContain(
	'["d", function() { return __var_d; }, function(value) { __var_d = value; }]'
);
expect(source).toContain(
	"Object.defineProperty(__proxy, item[0], { configurable: true, enumerable: true, get: item[1], set: item[2] })"
);
expect(source).toContain(
	'Object.defineProperty(__webpack_require__, item[0], { configurable: true, enumerable: true, get: function() { return __proxy[item[0]]; }, set: function(value) { __proxy[item[0]] = value; } })'
);
expect(source).toContain("for (var i = 0; i < __bridge.length; i++)");
expect(source).not.toContain(".forEach(");
expect(source).toContain(
	'["p", function() { return __var_p; }, function(value) { __var_p = value; }]'
);
expect(source).toContain(
	'["nc", function() { return __var_nc; }, function(value) { __var_nc = value; }]'
);
expect(source).toContain(
	'["b", function() { return __var_b; }, function(value) { __var_b = value; }]'
);
expect(source).not.toContain('["d", __var_d]');
expect(source).not.toContain('["nc", __var_nc]');
expect(source).not.toContain('["b", __var_b]');
expect(source).toContain("__webpack_require__.d = function runtimeWriteBridge()");
expect(source).toContain(
	'__webpack_require__["p"] = "/static-computed-public-path/"'
);
expect(source).toContain("__rspack_runtime.nc = \"runtime-proxy-nonce\"");
expect(source).toContain("__rspack_runtime.b = \"runtime-proxy-base-uri\"");
expect(source).toContain("function shadow(__webpack_require__)");
expect(source).toContain("__webpack_require__.t = function shadowedRequireWrite()");
expect(source).not.toContain(
	'Object.defineProperty(__webpack_require__, "t"'
);
expect(source).not.toContain(
	'Object.defineProperty(__webpack_require__, "d"'
);
