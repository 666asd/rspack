/** @type {import("@rspack/core").Configuration} */
module.exports = {
  entry: {
    index: './index.js',
  },
  output: {
    filename: `[name].js`,
    module: true,
    library: { type: 'modern-module' },
    iife: false,
    chunkFormat: 'module',
  },
  externalsType: 'module-import',
  externals: 'external-module',
  optimization: {
    runtimeChunk: false,
  },
  plugins: [
    function () {
      /**
       * @param {import("@rspack/core").Compilation} compilation compilation
       * @returns {void}
       */
      const handler = (compilation) => {
        compilation.hooks.afterProcessAssets.tap('testcase', (assets) => {
          const bundle = Object.values(assets)[0]._value;
          expect(bundle).toContain(
            `import external_module, { namedImport } from "external-module";`,
          );
          expect(bundle).toContain('external_module as defaultImport');
          expect(bundle).toContain('cjsInterop');
          expect(bundle).toContain('namedImport');
        });
      };
      this.hooks.compilation.tap('testcase', handler);
    },
  ],
};
