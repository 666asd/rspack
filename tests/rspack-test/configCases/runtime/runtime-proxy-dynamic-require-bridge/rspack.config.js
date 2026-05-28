module.exports = {
  mode: 'development',
  experiments: {
    runtimeMode: 'rspack',
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
