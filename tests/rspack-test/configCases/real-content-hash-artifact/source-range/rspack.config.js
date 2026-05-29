module.exports = {
  mode: 'production',
  entry: './index.js',
  output: {
    filename: '[name].[contenthash:8].js',
    chunkFilename: '[name].[contenthash:8].js',
  },
  optimization: {
    realContentHash: true,
  },
};
