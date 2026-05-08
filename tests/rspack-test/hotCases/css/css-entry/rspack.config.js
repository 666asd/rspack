/** @type {import("@rspack/core").Configuration} */
module.exports = {
  mode: 'development',
  devtool: false,
  entry: {
    'css-entry': './entry.css',
    main: './index.js',
  },
  output: {
    filename: '[name].js',
    cssFilename: '[name].css',
  },
  experiments: {
    css: true,
  },
  node: {
    __dirname: false,
  },
};
