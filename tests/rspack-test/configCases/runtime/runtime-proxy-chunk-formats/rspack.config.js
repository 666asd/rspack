module.exports = [
  {
    name: 'web',
    target: 'web',
    experiments: { runtimeRequirementsProxy: true },
    entry: './index.js',
    output: { filename: 'web.js' },
  },
  {
    name: 'commonjs',
    target: 'node',
    experiments: { runtimeRequirementsProxy: true },
    entry: './index.js',
    output: { filename: 'commonjs.js', chunkFormat: 'commonjs' },
  },
  {
    name: 'module',
    target: 'web',
    experiments: { runtimeRequirementsProxy: true },
    entry: './index.js',
    output: { filename: 'module.js', module: true, chunkFormat: 'module' },
  },
];
