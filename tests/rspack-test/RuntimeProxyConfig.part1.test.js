const path = require("path");
const { describeByWalk, createConfigCase } = require("@rspack/test-tools");

describeByWalk(
	__filename,
	(name, src, dist) => {
		createConfigCase(name, src, dist);
	},
	{
		source: path.join(__dirname, "configCases"),
		dist: path.resolve(__dirname, "./js/runtime-proxy-config"),
		exclude: [
			// Exclude e-z and non-ascii
			/^[e-z]/,
			/^[^a-d]/,
			/^runtime\/runtime-proxy-disabled$/
		]
	}
);
