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
			// Exclude a-d, p-z and non-ascii
			/^[a-d]/,
			/^[p-z]/,
			/^[^e-o]/,
			/^runtime\/runtime-proxy-disabled$/
		]
	}
);
