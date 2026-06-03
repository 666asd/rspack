const path = require("path");
const { describeByWalk, createConfigCase } = require("@rspack/test-tools");

process.env.RSPACK_TEST_RUNTIME_MODE_RSPACK = "true";

describeByWalk(
	__filename,
	(name, src, dist) => {
		createConfigCase(name, src, dist);
	},
	{
		source: path.join(__dirname, "configCases/runtime"),
		dist: path.resolve(__dirname, "./js/runtime-mode-config"),
		level: 1,
		exclude: [/^(?!runtime-mode-)/]
	}
);
