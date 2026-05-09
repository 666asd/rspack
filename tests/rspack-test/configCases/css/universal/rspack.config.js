'use strict';

/** @type {import("@rspack/core").Configuration[]} */
module.exports = [
  {
    name: 'web',
    target: 'web',
    devtool: false,
    mode: 'development',
    output: {
      module: true,
      chunkFormat: 'module',
    },
    experiments: {
      css: true,
    },
  },
  {
    name: 'node',
    target: 'node',
    devtool: false,
    mode: 'development',
    output: {
      module: true,
      chunkFormat: 'module',
    },
    experiments: {
      css: true,
    },
  },
];
