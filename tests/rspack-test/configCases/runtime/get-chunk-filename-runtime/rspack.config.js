/** @type {import("@rspack/core").Configuration} */
module.exports = {
  entry: {
    a: './a.js',
    b: './b.js',
  },
  output: {
    filename: '[name].js',
    chunkFilename: '[runtime].[contenthash:8].[name].js',
  },
};
