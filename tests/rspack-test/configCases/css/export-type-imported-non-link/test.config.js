"use strict";

const fs = require("fs");
const path = require("path");

module.exports = {
	findBundle(i, options) {
		const css = fs.readFileSync(
			path.resolve(options.output.path, "bundle0.css"),
			"utf-8"
		);

		if (
			!css.includes(".link-parent") ||
			!css.includes(".link-parent-with-conditions") ||
			!css.includes(".link-parent-with-same-child-conditions") ||
			!css.includes(".text-child") ||
			!css.includes(".conditional-child") ||
			!css.includes(".sheet-child") ||
			!css.includes(".style-child") ||
			!css.includes("@media print{") ||
			!css.includes("  @supports (display: grid) {") ||
			!css.includes("    @layer theme {") ||
			css.includes("@charset") ||
			css.includes("@media screen") ||
			css.includes("@import")
		) {
			throw new Error("non-link CSS child should be rendered into link CSS asset");
		}
		const textChildOccurrences = css.match(/\.text-child/g) || [];
		if (textChildOccurrences.length < 2) {
			throw new Error(
				"same CSS child should be rendered once per distinct import condition"
			);
		}

		return "./bundle0.js";
	},
	moduleScope(scope) {
		const link = scope.window.document.createElement("link");
		link.rel = "stylesheet";
		link.href = "bundle0.css";
		scope.window.document.head.appendChild(link);
	}
};
