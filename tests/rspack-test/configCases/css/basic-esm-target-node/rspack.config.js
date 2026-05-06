/** @type {import("@rspack/core").Configuration} */
module.exports = {
  target: 'node',
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
