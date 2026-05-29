const {
  Compilation,
  sources: { RawSource },
} = require('@rspack/core');

const getContentHash = (asset) => {
  const contenthash = asset.info.contenthash;
  if (Array.isArray(contenthash)) return contenthash[0];
  return contenthash;
};

const findReferencedAsset = (compilation) =>
  compilation
    .getAssets()
    .find((asset) => /^referenced\.[a-f0-9]{8}\.js$/.test(asset.name));

const byteOffset = (source, stringIndex) =>
  Buffer.byteLength(source.slice(0, stringIndex));

class InvalidRangeManifestPlugin {
  apply(compiler) {
    compiler.hooks.thisCompilation.tap(
      'InvalidRangeManifestPlugin',
      (compilation) => {
        compilation.hooks.processAssets.tap(
          {
            name: 'InvalidRangeManifestPlugin',
            stage: Compilation.PROCESS_ASSETS_STAGE_ADDITIONS,
          },
          () => {
            const asset = findReferencedAsset(compilation);
            const referencedHash = getContentHash(asset);
            compilation.emitAsset(
              'manifest.json',
              new RawSource(
                JSON.stringify({ prefix: 'π:', hash: referencedHash }),
              ),
            );
          },
        );

        compilation.hooks.processAssets.tap(
          {
            name: 'InvalidRangeManifestPlugin',
            stage: Compilation.PROCESS_ASSETS_STAGE_OPTIMIZE_HASH - 1,
          },
          () => {
            const asset = findReferencedAsset(compilation);
            const referencedHash = getContentHash(asset);
            const source = compilation
              .getAsset('manifest.json')
              .source.source();
            const start = source.indexOf(referencedHash);

            compilation.recordRealContentHashReference({
              asset: 'manifest.json',
              referencedHash,
              range: [
                byteOffset(source, start) + 0.5,
                byteOffset(source, start + referencedHash.length),
              ],
            });
          },
        );
      },
    );
  }
}

module.exports = {
  mode: 'production',
  entry: {
    main: './index.js',
    referenced: './referenced.js',
  },
  output: {
    filename: '[name].[contenthash:8].js',
  },
  optimization: {
    realContentHash: true,
  },
  plugins: [new InvalidRangeManifestPlugin()],
};
