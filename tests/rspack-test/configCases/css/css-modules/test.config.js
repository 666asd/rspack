"use strict";

const fs = require("fs");

const findDeterministicBundle = (dir, i) => {
	const re = new RegExp(`^\\d+\\.bundle${i}\\.js$`);
	const found = fs.readdirSync(dir).find((f) => re.test(f));
	if (!found) throw new Error(`No deterministic bundle found for index ${i}`);
	return found;
};

module.exports = {
	findBundle(i, options) {
		return i === 0
			? ["./use-style_js.bundle0.js", "./bundle0.js"]
			: [`./${findDeterministicBundle(options.output.path, i)}`, "./bundle1.js"];
	}
};
