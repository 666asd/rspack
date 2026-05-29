module.exports = {
	mode: "production",
	entry: "./index.js",
	output: {
		filename: "[name].[contenthash].js",
		chunkFilename: "[name].[contenthash].js"
	},
	optimization: {
		realContentHash: true
	}
};
