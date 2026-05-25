const {
  experiments: { RstestPlugin },
} = require('@rspack/core');

const createConfig = (entry, filename, rstestOptions = {}) => ({
  entry,
  target: 'node',
  output: {
    filename,
  },
  plugins: [
    new RstestPlugin({
      injectModulePathName: false,
      hoistMockModule: false,
      importMetaPathName: false,
      manualMockRoot: __dirname,
      ...rstestOptions,
    }),
  ],
});

/** @type {import('@rspack/core').Configuration[]} */
module.exports = [
  createConfig('./index.js', 'enabled.js'),
  createConfig('./disabled.js', 'disabled.js', {
    definePropertyGettersCompat: false,
  }),
];
