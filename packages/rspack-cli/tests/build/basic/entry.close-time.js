const path = require('path');

module.exports = {
  mode: 'development',
  entry: './src/entry.js',
  output: {
    path: path.resolve(__dirname, 'dist/close-time'),
  },
  plugins: [
    {
      apply(compiler) {
        compiler.hooks.shutdown.tapAsync(
          'SlowShutdownForStatsTimePlugin',
          (callback) => setTimeout(callback, 1200),
        );
      },
    },
  ],
};
