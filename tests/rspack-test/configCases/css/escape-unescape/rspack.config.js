'use strict';

/** @type {import("@rspack/core").Configuration[]} */
module.exports = [
  {
    target: 'web',
    mode: 'development',
    module: {
      rules: [],
    },
    experiments: {
      css: true,
    },
  },
  {
    target: 'web',
    mode: 'production',
    module: {
      rules: [],
    },
    experiments: {
      css: true,
    },
  },
];
