const path = require("path");
const { describeByWalk, createHotCase } = require("@rspack/test-tools");
const tempDir = path.resolve(__dirname, "./js/temp/runtime-mode-hot-web");

process.env.RSPACK_TEST_RUNTIME_MODE_RSPACK = "true";

describeByWalk(
	__filename,
	(name, src, dist) => {
		createHotCase(name, src, dist, path.join(tempDir, name), "web");
	},
	{
		source: path.resolve(__dirname, "./hotCases/runtime"),
		dist: path.resolve(__dirname, "./js/runtime-mode-hot-web"),
		level: 1,
		exclude: [/^(?!accept$)/]
	}
);
