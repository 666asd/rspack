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

const byteRange = (source, start, end) => [
  byteOffset(source, start),
  byteOffset(source, end),
];

class ReferencedManifestPlugin {
  apply(compiler) {
    compiler.hooks.thisCompilation.tap(
      'ReferencedManifestPlugin',
      (compilation) => {
        compilation.hooks.processAssets.tap(
          {
            name: 'ReferencedManifestPlugin',
            stage: Compilation.PROCESS_ASSETS_STAGE_ADDITIONS,
          },
          () => {
            const asset = findReferencedAsset(compilation);
            const referencedHash = getContentHash(asset);
            compilation.emitAsset(
              'manifest.json',
              new RawSource(
                JSON.stringify(
                  {
                    prefix: 'π:',
                    file: asset.name,
                    hash: referencedHash,
                  },
                  null,
                  2,
                ),
              ),
            );
          },
        );

        compilation.hooks.processAssets.tap(
          {
            name: 'ReferencedManifestPlugin',
            stage: Compilation.PROCESS_ASSETS_STAGE_OPTIMIZE_HASH - 1,
          },
          () => {
            const asset = findReferencedAsset(compilation);
            const referencedHash = getContentHash(asset);
            const source = compilation
              .getAsset('manifest.json')
              .source.source();
            const finalSource = JSON.stringify(JSON.parse(source));

            compilation.updateAsset(
              'manifest.json',
              new RawSource(finalSource),
            );

            let start = finalSource.indexOf(referencedHash);
            while (start >= 0) {
              const end = start + referencedHash.length;
              compilation.recordRealContentHashReference({
                asset: 'manifest.json',
                referencedHash,
                range: byteRange(finalSource, start, end),
              });
              start = finalSource.indexOf(referencedHash, end);
            }
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
  plugins: [new ReferencedManifestPlugin()],
};
