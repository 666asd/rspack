'use strict';

/** @type {import("@rspack/core").Configuration[]} */
module.exports = [
  {
    name: 'web',
    mode: 'development',
    target: 'web',
    module: {
      rules: [
        {
          test: /\.svg$/,
          type: 'asset/bytes',
        },
      ],
    },
    experiments: {
      css: true,
    },
  },
  {
    name: 'node',
    mode: 'development',
    target: 'node',
    module: {
      rules: [
        {
          test: /\.svg$/,
          type: 'asset/bytes',
        },
      ],
    },
    experiments: {
      css: true,
    },
  },
  {
    name: 'universal',
    mode: 'development',
    target: ['web', 'node'],
    output: {
      module: true,
    },
    module: {
      rules: [
        {
          test: /\.svg$/,
          type: 'asset/bytes',
        },
      ],
    },
    experiments: {
      css: true,
    },
  },
];
