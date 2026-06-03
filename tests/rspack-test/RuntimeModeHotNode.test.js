const path = require("path");
const { describeByWalk, createHotCase } = require("@rspack/test-tools");
const tempDir = path.resolve(__dirname, "./js/temp/runtime-mode-hot-node");

const rspackRuntimeModeOptions = {
	experiments: {
		runtimeMode: "rspack"
	}
};
globalThis.__RSPACK_TEST_RUNTIME_MODE_RSPACK = true;

describeByWalk(
	__filename,
	(name, src, dist) => {
		createHotCase(
			name,
			src,
			dist,
			path.join(tempDir, name),
			"async-node",
			rspackRuntimeModeOptions
		);
	},
	{
		source: path.resolve(__dirname, "./hotCases"),
		dist: path.resolve(__dirname, "./js/runtime-mode-hot-node"),
		exclude: [/^css$/]
	}
);
