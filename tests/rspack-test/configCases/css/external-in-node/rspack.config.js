const path = require('path');

/** @type {import("@rspack/core").Configuration} */
module.exports = {
  context: path.join(__dirname, '../external'),
  entry: '../external-in-node/index.js',
  target: 'node',
  module: {
    rules: [],
  },
  experiments: {
    css: true,
  },
};
