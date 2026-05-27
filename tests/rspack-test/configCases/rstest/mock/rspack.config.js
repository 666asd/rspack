const path = require('path');
const {
  experiments: { RstestPlugin },
} = require('@rspack/core');

class RstestSimpleRuntimePlugin {
  constructor() {}

  apply(compiler) {
    const { RuntimeModule, RuntimeGlobals } = compiler.rspack;
    class RetestImportRuntimeModule extends RuntimeModule {
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
      throw new Error(\`Cannot find module '\${args[0]}'\`)
    }
    throw e;
  }
};

Object.keys(originalRequire).forEach(key => {
  __rspack_require[key] = originalRequire[key];
});

__rspack_require.rstest_original_modules = {};

__rspack_require.rstest_reset_modules = () => {
  const mockedIds = Object.keys(__rspack_require.rstest_original_modules)
  Object.keys(__rspack_module_cache).forEach(id => {
    // Do not reset mocks registry.
    if (!mockedIds.includes(id)) {
      delete __rspack_module_cache[id];
    }
  });
}

__rspack_require.rstest_unmock = (id) => {
  delete __rspack_module_cache[id]
}

__rspack_require.rstest_require_actual = __rspack_require.rstest_import_actual = (id) => {
  const originalModule = __rspack_require.rstest_original_modules[id];
  // Use fallback module if the module is not mocked.
  const fallbackMod = __rspack_require(id);
  return originalModule ? originalModule : fallbackMod;
}

__rspack_require.rstest_exec = async (id, modFactory) => {
  if (__rspack_module_cache) {
    let asyncFactory = __rspack_module_cache[id];
    if (asyncFactory && asyncFactory.constructor.name === 'AsyncFunction') {
      await asyncFactory();
    }
  }
};

__rspack_require.rstest_mock = (id, modFactory) => {
  let requiredModule = undefined
  try {
    requiredModule = __rspack_require(id);
  } catch {
    // TODO: non-resolved module
  } finally {
    __rspack_require.rstest_original_modules[id] = requiredModule;
  }
  if (typeof modFactory === 'string' || typeof modFactory === 'number') {
    __rspack_module_cache[id] = { exports: __rspack_require(modFactory) };
  } else if (typeof modFactory === 'function') {
    const finalModFactory = function (
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

__rspack_require.rstest_mock_require = __rspack_require.rstest_mock;

__rspack_require.rstest_do_mock = (id, modFactory) => {
  let requiredModule = undefined
  try {
    requiredModule = __rspack_require(id);
  } catch {
    // TODO: non-resolved module
  } finally {
    __rspack_require.rstest_original_modules[id] = requiredModule;
  }
  if (typeof modFactory === 'string' || typeof modFactory === 'number') {
    __rspack_module_cache[id] = { exports: __rspack_require(modFactory) };
  } else if (typeof modFactory === 'function') {
    const exports = modFactory();
    __rspack_require.r(exports);
    __rspack_module_cache[id] = { exports, id, loaded: true };
  }
};

__rspack_require.rstest_do_mock_require = __rspack_require.rstest_do_mock;

__rspack_require.rstest_hoisted = (fn) => {
  return fn();
};
`;
      }
    }

    compiler.hooks.thisCompilation.tap(
      'RstestSimpleRuntimePlugin',
      (compilation) => {
        compilation.hooks.additionalTreeRuntimeRequirements.tap(
          'RstestSimpleRuntimePlugin',
          (chunk) => {
            compilation.addRuntimeModule(
              chunk,
              new RetestImportRuntimeModule(),
            );
          },
        );
      },
    );
  }
}

const rstestEntry = (entry, rstestPluginOptions = {}) => {
  return {
    entry,
    target: 'node',
    node: {
      __filename: false,
      __dirname: false,
    },
    optimization: {
      // TODO: should only mark mocked modules as used.
      usedExports: false,
      mangleExports: false,
      concatenateModules: false,
      moduleIds: 'named',
    },
    module: {
      rules: [
        {
          test: /\.js$/,
          loader: path.resolve(__dirname, './importActualLoader.mjs'),
          with: {
            rstest: 'importActual',
          },
        },
      ],
    },
    plugins: [
      new RstestSimpleRuntimePlugin(),
      new RstestPlugin({
        injectModulePathName: true,
        hoistMockModule: true,
        importMetaPathName: true,
        manualMockRoot: path.resolve(__dirname, '__mocks__'),
        ...rstestPluginOptions,
      }),
    ],
  };
};

/** @type {import("@rspack/core").Configuration} */
module.exports = [
  rstestEntry('./doMock.js'),
  rstestEntry('./mockFactory.js'),
  rstestEntry('./manualMock.js'),
  rstestEntry('./builtinManualMock.js'),
  rstestEntry('./nodeModulesManualMock.js'),
  rstestEntry('./directoryManualMock.js'),
  rstestEntry('./importActual.js'),
  rstestEntry('./importActualHoisted.js'),
  rstestEntry('./requireActual.js'),
  rstestEntry('./doMockRequire.js'),
  rstestEntry('./unmockRequire.js'),
  {
    entry: './test.js',
    target: 'node',
    node: {
      __filename: false,
      __dirname: false,
    },
    optimization: {
      mangleExports: false,
    },
  },
  rstestEntry('./mockFirstArgIsImport.js'),
  rstestEntry('./reExportMockedModule.js'),
  rstestEntry('./reExportDoMockedModule.js'),
  rstestEntry('./reExportTripleMockedModule.js'),
  rstestEntry('./reExportTripleDoMockedModule.js'),
  rstestEntry('./globals/importActual.js'),
  rstestEntry('./globals-false/importActual.js', { globals: false }),
  {
    ...rstestEntry('./mainFilesManualMock.js'),
    resolve: {
      mainFiles: ['main'],
    },
  },
  rstestEntry('./filePrecedenceManualMock.js'),
  {
    ...rstestEntry('./hoisted.js'),
    externals: {
      '@rstest/core': 'global @rstest/core',
    },
  },
];
