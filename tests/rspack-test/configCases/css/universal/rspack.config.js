/** @type {import("@rspack/core").Configuration} */
module.exports = {
  output: {
    module: true,
  },
  target: ['web', 'node'],
  mode: 'development',
  module: {
    rules: [],
  },
  experiments: {
    css: true,
  },
};
