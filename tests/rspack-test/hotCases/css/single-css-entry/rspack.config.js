/** @type {import("@rspack/core").Configuration} */
module.exports = {
  mode: 'development',
  devtool: false,
  entry: ['./index.js', './index.css'],
  experiments: {
    css: true,
  },
};
