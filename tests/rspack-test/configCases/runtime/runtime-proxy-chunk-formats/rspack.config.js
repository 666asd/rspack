module.exports = [
  {
    name: 'web',
    target: 'web',
    experiments: { runtimeMode: 'rspack' },
    entry: './index.js',
    output: { filename: 'web.js' },
  },
  {
    name: 'commonjs',
    target: 'node',
    experiments: { runtimeMode: 'rspack' },
    entry: './index.js',
    output: { filename: 'commonjs.js', chunkFormat: 'commonjs' },
  },
  {
    name: 'module',
    target: 'web',
    experiments: { runtimeMode: 'rspack' },
    entry: './index.js',
    output: { filename: 'module.js', module: true, chunkFormat: 'module' },
  },
];
