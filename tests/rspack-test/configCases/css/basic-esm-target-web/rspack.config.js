/** @type {import("@rspack/core").Configuration} */
module.exports = {
  target: 'web',
  mode: 'development',
  module: {
    rules: [],
  },
  output: {
    module: true,
    chunkFormat: 'module',
  },
  experiments: {
    css: true,
  },
};
