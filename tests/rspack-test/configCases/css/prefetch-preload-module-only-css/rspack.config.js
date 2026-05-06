/** @type {import("@rspack/core").Configuration} */
module.exports = {
  entry: './index.mjs',
  module: {
    rules: [],
  },
  name: 'esm',
  target: 'web',
  output: {
    module: true,
    publicPath: '',
    filename: 'bundle0.mjs',
    chunkFilename: '[name].mjs',
    crossOriginLoading: 'anonymous',
    chunkFormat: 'module',
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
