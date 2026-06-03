const path = require("path");
const { describeByWalk, createConfigCase } = require("@rspack/test-tools");

const rspackRuntimeModeOptions = {
	experiments: {
		runtimeMode: "rspack"
	}
};
globalThis.__RSPACK_TEST_RUNTIME_MODE_RSPACK = true;

describeByWalk(
	__filename,
	(name, src, dist) => {
		createConfigCase(name, src, dist, rspackRuntimeModeOptions);
	},
	{
		source: path.join(__dirname, "configCases/runtime"),
		dist: path.resolve(__dirname, "./js/runtime-mode-config"),
		level: 1,
		exclude: [/^(?!runtime-mode-)/]
	}
);
