const { DefinePlugin } = require('@rspack/core');

/** @type {import("@rspack/core").Configuration[]} */
module.exports = [
  {
    experiments: {
      runtimeMode: 'compatibility',
    },
    optimization: {
      concatenateModules: false,
      innerGraph: false,
      usedExports: false,
    },
    output: {
      filename: 'compat.js',
      iife: false,
    },
    plugins: [
      new DefinePlugin({
        DEFINED_DEP: '__webpack_require__(16)',
      }),
    ],
  },
  {
    experiments: {
      runtimeMode: 'rspack',
    },
    optimization: {
      concatenateModules: false,
      innerGraph: false,
      usedExports: false,
    },
    output: {
      filename: 'rspack-only.js',
      iife: false,
    },
    plugins: [
      new DefinePlugin({
        DEFINED_DEP: '__rspack_require(16)',
      }),
    ],
  },
];
