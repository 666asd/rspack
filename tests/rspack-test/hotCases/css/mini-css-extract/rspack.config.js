const { CssExtractRspackPlugin } = require('@rspack/core');

/** @type {import("@rspack/core").Configuration} */
module.exports = {
  mode: 'development',
  devtool: false,
  entry: {
    main: './a.js',
    b: { import: './b.js', dependOn: 'main' },
  },
  experiments: {
    css: false,
  },
  module: {
    rules: [
      {
        test: /\.css$/,
        type: 'javascript/auto',
        use: [
          {
            loader: CssExtractRspackPlugin.loader,
          },
          {
            loader: 'css-loader',
            options: {
              esModule: true,
              modules: {
                namedExport: false,
                localIdentName: '[name]',
              },
            },
          },
        ],
      },
    ],
  },
  output: {
    filename: '[name].js',
    cssChunkFilename: '[name].css',
  },
  target: 'web',
  node: {
    __dirname: false,
  },
  plugins: [
    new CssExtractRspackPlugin({
      experimentalUseImportModule: true,
      filename: '[name].css',
    }),
  ],
};
