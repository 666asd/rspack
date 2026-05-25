module.exports = {
  mode: 'development',
  experiments: {
    runtimeRequirementsProxy: true,
  },
  output: {
    environment: {
      arrowFunction: false,
      const: false,
    },
  },
  optimization: {
    concatenateModules: false,
  },
};
