/** @type {import("@rspack/core").Configuration} */
module.exports = [
  {
    target: 'web',
    mode: 'development',
    module: {
      generator: {
        css: {
          exportsOnly: true,
        },
        'css/module': {
          exportsOnly: false,
        },
      },
      rules: [
        {
          resourceQuery: /\?module/,
          type: 'css/module',
        },
      ],
    },
    node: {
      __dirname: false,
    },
    experiments: {
      css: true,
    },
  },
];
