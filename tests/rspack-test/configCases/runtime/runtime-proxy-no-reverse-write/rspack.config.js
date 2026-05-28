const { RuntimeModule, RuntimeGlobals } = require('@rspack/core');

class ReverseWriteRuntimeModule extends RuntimeModule {
  constructor() {
    super('reverse write runtime module');
  }

  generate() {
    return `
			__webpack_require__.d = function changedByCustomRuntimeModule() {};
		`;
  }
}

class ReverseWritePlugin {
  apply(compiler) {
    compiler.hooks.compilation.tap('ReverseWritePlugin', (compilation) => {
      compilation.hooks.additionalTreeRuntimeRequirements.tap(
        'ReverseWritePlugin',
        (chunk, set) => {
          set.add(RuntimeGlobals.definePropertyGetters);
          compilation.addRuntimeModule(chunk, new ReverseWriteRuntimeModule());
        },
      );
    });
  }
}

module.exports = {
  experiments: {
    runtimeMode: 'rspack',
  },
  plugins: [new ReverseWritePlugin()],
};
