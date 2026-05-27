const path = require('path');
const {
  experiments: { RstestPlugin },
} = require('@rspack/core');

class RstestSimpleRuntimePlugin {
  apply(compiler) {
    const { RuntimeModule } = compiler.rspack;

    class RstestRuntimeModule extends RuntimeModule {
      constructor() {
        super('rstest runtime');
      }

      generate() {
        return `
if (typeof __rspack_require === 'undefined') {
  return;
}

const originalRequire = __rspack_require;
__rspack_require = function(...args) {
  try {
    return originalRequire(...args);
  } catch (e) {
    const errMsg = e.message ?? e.toString();
    if (errMsg.includes('__rspack_modules[moduleId] is not a function')) {
      throw new Error(\`Cannot find module '\${args[0]}'\`);
    }
    throw e;
  }
};

Object.keys(originalRequire).forEach(key => {
  __rspack_require[key] = originalRequire[key];
});

__rspack_require.rstest_original_modules = {};

__rspack_require.rstest_mock = (id, modFactory) => {
  let requiredModule = undefined;
  try {
    requiredModule = __rspack_require(id);
  } catch {}
  finally {
    __rspack_require.rstest_original_modules[id] = requiredModule;
  }

  if (typeof modFactory === 'string' || typeof modFactory === 'number') {
    __rspack_module_cache[id] = { exports: __rspack_require(modFactory) };
  } else if (typeof modFactory === 'function') {
    const finalModFactory = function(
      __unused_webpack_module,
      __rspack_exports,
      __rspack_require,
    ) {
      __rspack_require.r(__rspack_exports);
      const res = modFactory();
      for (const key in res) {
        __rspack_require.d(__rspack_exports, {
          [key]: () => res[key],
        });
      }
    };

    __rspack_modules[id] = finalModFactory;
    delete __rspack_module_cache[id];
  }
};

__rspack_require.rstest_hoisted = fn => fn();
`;
      }
    }

    compiler.hooks.thisCompilation.tap(
      'RstestSimpleRuntimePlugin',
      (compilation) => {
        compilation.hooks.additionalTreeRuntimeRequirements.tap(
          'RstestSimpleRuntimePlugin',
          (chunk) => {
            compilation.addRuntimeModule(chunk, new RstestRuntimeModule());
          },
        );
      },
    );
  }
}

/** @type {import("@rspack/core").Configuration} */
module.exports = {
  context: __dirname,
  target: 'node',
  node: {
    __filename: false,
    __dirname: false,
  },
  cache: {
    type: 'persistent',
  },
  optimization: {
    usedExports: false,
    mangleExports: false,
    concatenateModules: false,
    moduleIds: 'named',
  },
  output: {
    library: { type: 'commonjs2' },
  },
  externals: {
    'virtual-module': 'node-commonjs virtual-module1',
  },
  plugins: [
    new RstestSimpleRuntimePlugin(),
    new RstestPlugin({
      injectModulePathName: true,
      hoistMockModule: true,
      importMetaPathName: true,
      manualMockRoot: path.resolve(__dirname, '__mocks__'),
      globals: true,
    }),
  ],
};
