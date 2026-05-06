/** @type {import("@rspack/core").Configuration} */
module.exports = {
  target: 'web',
  output: {
    chunkFilename: '[name].js',
  },
  module: {
    rules: [],
  },
  experiments: {
    css: true,
  },
};
