const fs = require('fs');
const path = require('path');

const outputPath = path.resolve(__dirname, 'dist/close-time');
const doneTimePath = path.resolve(outputPath, 'done-time.json');

module.exports = {
  mode: 'development',
  entry: './src/entry.js',
  output: {
    path: outputPath,
  },
  plugins: [
    {
      apply(compiler) {
        compiler.hooks.done.tap('RecordStatsTimePlugin', (stats) => {
          fs.mkdirSync(outputPath, { recursive: true });
          fs.writeFileSync(
            doneTimePath,
            JSON.stringify(stats.toJson({ all: false, timings: true })),
          );
        });
        compiler.hooks.shutdown.tapAsync(
          'SlowShutdownForStatsTimePlugin',
          (callback) => setTimeout(callback, 1200),
        );
      },
    },
  ],
};
