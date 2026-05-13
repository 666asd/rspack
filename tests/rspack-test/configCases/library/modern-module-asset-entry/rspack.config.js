const assert = require('assert');

/** @type {import("@rspack/core").Configuration} */
module.exports = {
  context: __dirname,
  entry: {
    index: './img.png',
  },
  output: {
    filename: `[name].js`,
    chunkFilename: `async.js`,
    module: true,
    library: {
      type: 'modern-module',
    },
    iife: false,
    chunkFormat: 'module',
    chunkLoading: 'import',
  },
  module: {
    rules: [
      {
        test: /\.png$/,
        type: 'asset/resource',
        generator: {
          filename: 'static/img/[name].png',
          importMode: 'preserve',
        },
      },
    ],
  },
  optimization: {
    concatenateModules: true,
    avoidEntryIife: true,
    minimize: false,
  },
  plugins: [
    new (class {
      apply(compiler) {
        compiler.hooks.compilation.tap('MyPlugin', (compilation) => {
          compilation.hooks.processAssets.tap('MyPlugin', (assets) => {
            let list = Object.keys(assets);
            const js = list.find((item) => item.endsWith('js'));
            const jsContent = assets[js].source().toString();

            const preseveImport = jsContent.match(
              /import\s+([A-Za-z_$][\w$]*)\s+from ['"]\.\/static\/img\/img\.png['"]/,
            );
            assert(preseveImport);
            const hasExports = new RegExp(
              `export\\s+default\\s+${preseveImport[1]}\\b`,
            ).test(jsContent);
            assert(hasExports);
          });
        });
      }
    })(),
  ],
};
