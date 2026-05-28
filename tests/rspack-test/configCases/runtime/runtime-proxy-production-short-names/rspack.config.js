module.exports = {
  mode: 'production',
  experiments: {
    runtimeMode: 'rspack',
  },
  optimization: {
    concatenateModules: false,
    minimize: false,
  },
};
