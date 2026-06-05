const {
  experiments: { RsdoctorPlugin },
} = require('@rspack/core');
const path = require('path');

function normalizeRequest(request) {
  return request.replaceAll('\\', '/');
}

/** @type {import("@rspack/core").Configuration} */
module.exports = {
  mode: 'production',
  optimization: {
    concatenateModules: false,
    usedExports: true,
  },
  plugins: [
    new RsdoctorPlugin({
      moduleGraphFeatures: ['graph'],
      chunkGraphFeatures: false,
      exportUsageGraph: true,
    }),
    {
      apply(compiler) {
        let exportUsageGraphCalled = false;
        compiler.hooks.compilation.tap(
          'TestPlugin::ExportUsageGraph',
          (compilation) => {
            const hooks = RsdoctorPlugin.getCompilationHooks(compilation);
            hooks.exportUsageGraph.tap(
              'TestPlugin::ExportUsageGraph',
              (exportUsageGraph) => {
                exportUsageGraphCalled = true;
                const edges = exportUsageGraph.exportUsageEdges
                  .map((edge) => ({
                    originModulePath: normalizeRequest(edge.originModulePath),
                    originExport: edge.originExport,
                    targetModulePath: normalizeRequest(edge.targetModulePath),
                    targetExport: edge.targetExport,
                    active: edge.active,
                  }))
                  .sort((a, b) =>
                    `${a.originModulePath}:${a.originExport}:${a.targetModulePath}:${a.targetExport}` >
                    `${b.originModulePath}:${b.originExport}:${b.targetModulePath}:${b.targetExport}`
                      ? 1
                      : -1,
                  );

                expect(edges).toContainEqual({
                  originModulePath: normalizeRequest(
                    path.join(__dirname, 'index.js'),
                  ),
                  originExport: undefined,
                  targetModulePath: normalizeRequest(
                    path.join(__dirname, 'lib.js'),
                  ),
                  targetExport: ['foo'],
                  active: true,
                });
                expect(edges).toContainEqual({
                  originModulePath: normalizeRequest(
                    path.join(__dirname, 'lib.js'),
                  ),
                  originExport: ['foo'],
                  targetModulePath: normalizeRequest(
                    path.join(__dirname, 'shared.js'),
                  ),
                  targetExport: ['bar'],
                  active: true,
                });
                expect(edges).not.toContainEqual({
                  originModulePath: normalizeRequest(
                    path.join(__dirname, 'lib.js'),
                  ),
                  originExport: ['unusedFoo'],
                  targetModulePath: normalizeRequest(
                    path.join(__dirname, 'shared.js'),
                  ),
                  targetExport: ['unused'],
                  active: true,
                });
              },
            );
          },
        );
        compiler.hooks.done.tap('TestPlugin::ExportUsageGraph', () => {
          expect(exportUsageGraphCalled).toBe(true);
        });
      },
    },
  ],
};
