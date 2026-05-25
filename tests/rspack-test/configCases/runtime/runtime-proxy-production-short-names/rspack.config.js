module.exports = {
  mode: 'production',
  experiments: {
    runtimeRequirementsProxy: true,
  },
  optimization: {
    concatenateModules: false,
    minimize: false,
  },
};
