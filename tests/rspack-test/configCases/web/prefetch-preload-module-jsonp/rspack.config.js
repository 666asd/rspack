/** @type {import("@rspack/core").Configuration} */
module.exports = {
  entry: './index.mjs',
  module: {
    rules: [],
  },
  name: 'esm',
  target: 'web',
  output: {
    publicPath: '',
    module: true,
    filename: 'bundle0.mjs',
    chunkFilename: '[name].js',
    crossOriginLoading: 'anonymous',
    chunkFormat: 'array-push',
  },
  performance: {
    hints: false,
  },
  optimization: {
    minimize: false,
  },
  experiments: {
    css: true,
  },
};
