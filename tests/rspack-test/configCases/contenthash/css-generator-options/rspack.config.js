const common = {
  target: 'web',
  optimization: {
    realContentHash: false,
  },
};

/** @type {import("@rspack/core").Configuration[]} */
module.exports = [
  {
    ...common,
    output: {
      filename: 'bundle0.[contenthash].js',
      chunkFilename: 'css0/[name].[contenthash].js',
      cssChunkFilename: 'css0/[name].[contenthash].css',
    },
    module: {
      rules: [
        {
          test: /\.css$/,
          type: 'css/module',
        },
      ],
    },
    experiments: {
      css: true,
    },
  },
  {
    ...common,
    output: {
      filename: 'bundle1.[contenthash].js',
      chunkFilename: 'css1/[name].[contenthash].js',
      cssChunkFilename: 'css1/[name].[contenthash].css',
    },
    module: {
      rules: [
        {
          test: /\.css$/,
          type: 'css/module',
          generator: {
            exportsConvention: 'camel-case',
          },
        },
      ],
    },
    experiments: {
      css: true,
    },
  },
  {
    ...common,
    output: {
      filename: 'bundle2.[contenthash].js',
      chunkFilename: 'css2/[name].[contenthash].js',
      cssChunkFilename: 'css2/[name].[contenthash].css',
    },
    module: {
      rules: [
        {
          test: /\.css$/,
          type: 'css/module',
          generator: {
            exportsConvention: 'camel-case-only',
          },
        },
      ],
    },
    experiments: {
      css: true,
    },
  },
  // MAYBE: support function type of exportsConvention
  {
    ...common,
    output: {
      filename: 'bundle3.[contenthash].js',
      chunkFilename: 'css3/[name].[contenthash].js',
      cssChunkFilename: 'css3/[name].[contenthash].css',
    },
    module: {
      rules: [
        {
          test: /\.css$/,
          type: 'css/module',
          generator: {
            // exportsConvention: name => name.toUpperCase()
          },
        },
      ],
    },
    experiments: {
      css: true,
    },
  },
  {
    ...common,
    output: {
      filename: 'bundle4.[contenthash].js',
      chunkFilename: 'css4/[name].[contenthash].js',
      cssChunkFilename: 'css4/[name].[contenthash].css',
    },
    module: {
      rules: [
        {
          test: /\.css$/,
          type: 'css/module',
          generator: {
            localIdentName: '[hash]-[local]',
          },
        },
      ],
    },
    experiments: {
      css: true,
    },
  },
  {
    ...common,
    output: {
      filename: 'bundle5.[contenthash].js',
      chunkFilename: 'css5/[name].[contenthash].js',
      cssChunkFilename: 'css5/[name].[contenthash].css',
    },
    module: {
      rules: [
        {
          test: /\.css$/,
          type: 'css/module',
          generator: {
            localIdentName: '[path][name][ext]__[local]',
          },
        },
      ],
    },
    experiments: {
      css: true,
    },
  },
  {
    ...common,
    output: {
      filename: 'bundle6.[contenthash].js',
      chunkFilename: 'css6/[name].[contenthash].js',
      cssChunkFilename: 'css6/[name].[contenthash].css',
    },
    module: {
      rules: [
        {
          test: /\.css$/,
          type: 'css/module',
          generator: {
            esModule: false,
          },
        },
      ],
    },
    experiments: {
      css: true,
    },
  },
];
