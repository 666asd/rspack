const { sources } = require('@rspack/core');

class UntrackedContentHashPlugin {
  apply(compiler) {
    compiler.hooks.thisCompilation.tap(
      'UntrackedContentHashPlugin',
      (compilation) => {
        compilation.hooks.processAssets.tap(
          {
            name: 'UntrackedContentHashPlugin',
            stage: compiler.webpack.Compilation.PROCESS_ASSETS_STAGE_ADDITIONS,
          },
          () => {
            compilation.emitAsset(
              'untracked.aaaa.js',
              new sources.RawSource("console.log('aaaa');"),
              { contenthash: 'aaaa' },
            );
          },
        );
      },
    );
  }
}

module.exports = {
  mode: 'production',
  entry: './index.js',
  optimization: { realContentHash: true },
  plugins: [new UntrackedContentHashPlugin()],
};
