/** @type {import("@rspack/core").Configuration} */
module.exports = {
  target: 'web',
  output: {
    chunkFilename: '[name].js',
    crossOriginLoading: 'anonymous',
  },
  module: {
    rules: [],
  },
  optimization: {
    minimize: false,
    splitChunks: {
      minSize: 1,
    },
  },
  experiments: {
    css: true,
  },
};
