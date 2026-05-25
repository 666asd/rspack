const { RuntimeModule, RuntimeGlobals } = require('@rspack/core');

class ReadDefineGetterRuntimeModule extends RuntimeModule {
  constructor() {
    super('read define getter runtime module');
  }

  generate() {
    return `
			var readResult = typeof __webpack_require__.d;
			__webpack_require__.runtimeProxyReadResult = readResult;
			__webpack_require__.d = function customDefinePropertyGetters(exports, definition) {
				__webpack_require__.runtimeProxyWriteResult = typeof definition;
			};
		`;
  }
}

class AddRuntimeModulePlugin {
  apply(compiler) {
    compiler.hooks.compilation.tap('AddRuntimeModulePlugin', (compilation) => {
      compilation.hooks.additionalTreeRuntimeRequirements.tap(
        'AddRuntimeModulePlugin',
        (chunk, set) => {
          set.add(RuntimeGlobals.definePropertyGetters);
          compilation.addRuntimeModule(
            chunk,
            new ReadDefineGetterRuntimeModule(),
          );
        },
      );
    });
  }
}

module.exports = {
  experiments: {
    runtimeRequirementsProxy: true,
  },
  plugins: [new AddRuntimeModulePlugin()],
};
