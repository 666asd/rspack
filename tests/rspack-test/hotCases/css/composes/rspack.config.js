/** @type {import("@rspack/core").Configuration} */
module.exports = {
  mode: 'development',
  devtool: false,
  output: {
    cssChunkFilename: '[name].css',
  },
  experiments: {
    css: true,
  },
};
