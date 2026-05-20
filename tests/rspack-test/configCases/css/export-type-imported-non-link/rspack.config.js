"use strict";

/** @type {import("@rspack/core").Configuration} */
module.exports = {
	target: "web",
	mode: "development",
	devtool: false,
	module: {
		rules: [
			{
				test: /link-parent(?:-with(?:-same-child)?-conditions)?\.css$/,
				type: "css/auto"
			},
			{
				test: /style-parent\.css$/,
				type: "css/module",
				parser: {
					exportType: "style"
				}
			},
			{
				test: /(?:text|conditional)-child\.css$/,
				type: "css/auto",
				parser: {
					exportType: "text"
				}
			},
			{
				test: /sheet-child\.css$/,
				type: "css/auto",
				parser: {
					exportType: "css-style-sheet"
				}
			},
			{
				test: /style-child\.css$/,
				type: "css/auto",
				parser: {
					exportType: "style"
				}
			}
		]
	},
	experiments: {
		css: true
	}
};
