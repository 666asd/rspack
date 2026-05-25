const {
  experiments: { RsdoctorPlugin },
} = require('@rspack/core');

/** @type {import("@rspack/core").Configuration} */
module.exports = {
  entry: {
    a: './a.js',
    b: './b.js',
  },
  output: {
    filename: '[name].js',
  },
  optimization: {
    chunkIds: 'named',
    moduleIds: 'named',
  },
  plugins: [
    new RsdoctorPlugin({
      moduleGraphFeatures: false,
      chunkGraphFeatures: ['graph', 'assets'],
    }),
    {
      apply(compiler) {
        compiler.hooks.compilation.tap('TestPlugin::Assets', (compilation) => {
          const hooks = RsdoctorPlugin.getCompilationHooks(compilation);
          hooks.assets.tap('TestPlugin::Assets', (data) => {
            const { assets } = data;
            expect(assets.length).toBe(4);
            const assetsInfo = assets.map((a) => ({
              size: a.size,
              path: a.path,
            }));
            assetsInfo.sort((a, b) => (a.path > b.path ? 1 : -1));
            expect(assetsInfo.map((asset) => asset.path)).toEqual([
              'a.js',
              'b.js',
              'c_js.js',
              'd_js.js',
            ]);
            expect(assetsInfo[0].size).toBe(assetsInfo[1].size);
            expect(assetsInfo[2].size).toBe(assetsInfo[3].size);
            expect(assetsInfo[0].size).toBeGreaterThan(assetsInfo[2].size);
          });
        });
      },
    },
    {
      apply(compiler) {
        compiler.hooks.compilation.tap(
          'TestPlugin::ChunkAssets',
          (compilation) => {
            const hooks = RsdoctorPlugin.getCompilationHooks(compilation);
            let chunks = [];
            hooks.chunkGraph.tap('TestPlugin::ChunkAssets', (data) => {
              chunks = data.chunks;
            });
            hooks.assets.tap('TestPlugin::Assets', (data) => {
              const { chunkAssets } = data;
              for (const chunk of chunks) {
                expect(
                  chunkAssets.find((a) => a.chunk === chunk.ukey).assets.length,
                ).toBe(1);
              }
            });
          },
        );
      },
    },
    {
      apply(compiler) {
        compiler.hooks.compilation.tap(
          'TestPlugin::EntrypointAssets',
          (compilation) => {
            const hooks = RsdoctorPlugin.getCompilationHooks(compilation);
            let entrypoints = [];
            hooks.chunkGraph.tap('TestPlugin::EntrypointAssets', (data) => {
              entrypoints = data.entrypoints;
            });
            hooks.assets.tap('TestPlugin::Assets', (data) => {
              const { entrypointAssets } = data;
              for (const ep of entrypointAssets) {
                expect(
                  entrypointAssets.find((a) => a.chunk === ep.ukey).assets
                    .length,
                ).toBe(1);
              }
            });
          },
        );
      },
    },
  ],
};
