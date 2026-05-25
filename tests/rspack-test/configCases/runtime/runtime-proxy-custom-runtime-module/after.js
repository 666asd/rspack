const fs = require("fs");
const path = require("path");

const source = fs.readFileSync(path.resolve(__dirname, "bundle0.js"), "utf-8");

expect(source).toContain('["d", () => __var_d, (value) => { __var_d = value; }]');
expect(source).toContain(
	"Object.defineProperty(__proxy, item[0], { configurable: true, enumerable: true, get: item[1], set: item[2] })"
);
expect(source).toContain(
	"Object.defineProperty(__webpack_require__, item[0], { configurable: true, enumerable: true, get: () => __proxy[item[0]], set: (value) => { __proxy[item[0]] = value; } })"
);
expect(source).not.toContain('Object.defineProperty(__webpack_require__, "d"');
expect(source).toContain("customDefinePropertyGetters");
expect(source).toContain("enumerable: true");
expect(source).toContain("for (let i = 0; i < __bridge.length; i++)");
expect(source.indexOf("Object.defineProperty(__proxy")).toBeLessThan(
	source.indexOf("read define getter runtime module")
);
