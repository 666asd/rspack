const { RuntimeModule, RuntimeGlobals } = require('@rspack/core');

class ReadDefineGetterRuntimeModule extends RuntimeModule {
  constructor() {
    super('read define getter runtime module');
  }

  generate() {
    return `
			var readResult = typeof __rspack_require.d;
			__rspack_require.runtimeProxyReadResult = readResult;
			__rspack_require.d = function customDefinePropertyGetters(exports, definition) {
				__rspack_require.runtimeProxyWriteResult = typeof definition;
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
    runtimeMode: 'rspack',
  },
  plugins: [new AddRuntimeModulePlugin()],
};
