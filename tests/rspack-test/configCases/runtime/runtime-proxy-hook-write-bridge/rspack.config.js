const { RuntimeGlobals } = require('@rspack/core');

class AddTreeRuntimeRequirementPlugin {
  apply(compiler) {
    compiler.hooks.compilation.tap(
      'AddTreeRuntimeRequirementPlugin',
      (compilation) => {
        compilation.hooks.additionalTreeRuntimeRequirements.tap(
          'AddTreeRuntimeRequirementPlugin',
          (_chunk, set) => {
            set.add(RuntimeGlobals.hasOwnProperty);
          },
        );
      },
    );
  }
}

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
  plugins: [new AddTreeRuntimeRequirementPlugin()],
};
