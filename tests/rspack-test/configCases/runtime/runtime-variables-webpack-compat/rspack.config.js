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
  },
];
