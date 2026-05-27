// @ts-nocheck
var __module_federation_bundler_runtime__,
  __module_federation_runtime_plugins__,
  __module_federation_remote_infos__,
  __module_federation_container_name__,
  __module_federation_share_strategy__,
  __module_federation_share_fallbacks__,
  __module_federation_library_type__;
export default function () {
  if (
    (__rspack_require.initializeSharingData ||
      __rspack_require.initializeExposesData) &&
    __rspack_require.federation
  ) {
    const override = (obj, key, value) => {
      if (!obj) return;
      if (obj[key]) obj[key] = value;
    };
    const merge = (obj, key, fn) => {
      const value = fn();
      if (Array.isArray(value)) {
        obj[key] ??= [];
        obj[key].push(...value);
      } else if (typeof value === 'object' && value !== null) {
        obj[key] ??= {};
        Object.assign(obj[key], value);
      }
    };
    const early = (obj, key, initial) => {
      obj[key] ??= initial();
    };

    const remotesLoadingChunkMapping =
      __rspack_require.remotesLoadingData?.chunkMapping ?? {};
    const remotesLoadingModuleIdToRemoteDataMapping =
      __rspack_require.remotesLoadingData?.moduleIdToRemoteDataMapping ?? {};
    const initializeSharingScopeToInitDataMapping =
      __rspack_require.initializeSharingData?.scopeToSharingDataMapping ?? {};
    const consumesLoadingChunkMapping =
      __rspack_require.consumesLoadingData?.chunkMapping ?? {};
    const consumesLoadingModuleToConsumeDataMapping =
      __rspack_require.consumesLoadingData?.moduleIdToConsumeDataMapping ?? {};
    const consumesLoadinginstalledModules = {};
    const initializeSharingInitPromises = [];
    const initializeSharingInitTokens = {};
    const containerShareScope =
      __rspack_require.initializeExposesData?.shareScope;

    for (const key in __module_federation_bundler_runtime__) {
      __rspack_require.federation[key] =
        __module_federation_bundler_runtime__[key];
    }

    early(
      __rspack_require.federation,
      'libraryType',
      () => __module_federation_library_type__,
    );
    early(
      __rspack_require.federation,
      'sharedFallback',
      () => __module_federation_share_fallbacks__,
    );
    const sharedFallback = __rspack_require.federation.sharedFallback;
    early(
      __rspack_require.federation,
      'consumesLoadingModuleToHandlerMapping',
      () => {
        const consumesLoadingModuleToHandlerMapping = {};
        for (let [moduleId, data] of Object.entries(
          consumesLoadingModuleToConsumeDataMapping,
        )) {
          consumesLoadingModuleToHandlerMapping[moduleId] = {
            getter: sharedFallback
              ? __rspack_require.federation.bundlerRuntime?.getSharedFallbackGetter(
                  {
                    shareKey: data.shareKey,
                    factory: data.fallback,
                    webpackRequire: __rspack_require,
                    libraryType: __rspack_require.federation.libraryType,
                  },
                )
              : data.fallback,
            treeShakingGetter: sharedFallback ? data.fallback : undefined,
            shareInfo: {
              shareConfig: {
                fixedDependencies: false,
                requiredVersion: data.requiredVersion,
                strictVersion: data.strictVersion,
                singleton: data.singleton,
                eager: data.eager,
              },
              scope: [data.shareScope],
            },
            shareKey: data.shareKey,
            treeShaking: __rspack_require.federation.sharedFallback
              ? {
                  get: data.fallback,
                  mode: data.treeShakingMode,
                }
              : undefined,
          };
        }
        return consumesLoadingModuleToHandlerMapping;
      },
    );

    early(__rspack_require.federation, 'initOptions', () => ({}));
    early(
      __rspack_require.federation.initOptions,
      'name',
      () => __module_federation_container_name__,
    );
    early(
      __rspack_require.federation.initOptions,
      'shareStrategy',
      () => __module_federation_share_strategy__,
    );
    early(__rspack_require.federation.initOptions, 'shared', () => {
      const shared = {};
      for (let [scope, stages] of Object.entries(
        initializeSharingScopeToInitDataMapping,
      )) {
        for (let stage of stages) {
          if (typeof stage === 'object' && stage !== null) {
            const {
              name,
              version,
              factory,
              eager,
              singleton,
              requiredVersion,
              strictVersion,
              treeShakingMode,
            } = stage;
            const shareConfig = {};
            const isValidValue = function (val) {
              return typeof val !== 'undefined';
            };
            if (isValidValue(singleton)) {
              shareConfig.singleton = singleton;
            }
            if (isValidValue(requiredVersion)) {
              shareConfig.requiredVersion = requiredVersion;
            }
            if (isValidValue(eager)) {
              shareConfig.eager = eager;
            }
            if (isValidValue(strictVersion)) {
              shareConfig.strictVersion = strictVersion;
            }
            const options = {
              version,
              scope: [scope],
              shareConfig,
              get: factory,
              treeShaking: treeShakingMode
                ? {
                    mode: treeShakingMode,
                  }
                : undefined,
            };
            if (shared[name]) {
              shared[name].push(options);
            } else {
              shared[name] = [options];
            }
          }
        }
      }
      return shared;
    });
    merge(__rspack_require.federation.initOptions, 'remotes', () =>
      Object.values(__module_federation_remote_infos__)
        .flat()
        .filter((remote) => remote.externalType === 'script'),
    );
    merge(
      __rspack_require.federation.initOptions,
      'plugins',
      () => __module_federation_runtime_plugins__,
    );

    early(__rspack_require.federation, 'bundlerRuntimeOptions', () => ({}));
    early(
      __rspack_require.federation.bundlerRuntimeOptions,
      'remotes',
      () => ({}),
    );
    early(
      __rspack_require.federation.bundlerRuntimeOptions.remotes,
      'chunkMapping',
      () => remotesLoadingChunkMapping,
    );
    early(
      __rspack_require.federation.bundlerRuntimeOptions.remotes,
      'remoteInfos',
      () => __module_federation_remote_infos__,
    );
    early(
      __rspack_require.federation.bundlerRuntimeOptions.remotes,
      'idToExternalAndNameMapping',
      () => {
        const remotesLoadingIdToExternalAndNameMappingMapping = {};
        for (let [moduleId, data] of Object.entries(
          remotesLoadingModuleIdToRemoteDataMapping,
        )) {
          remotesLoadingIdToExternalAndNameMappingMapping[moduleId] = [
            data.shareScope,
            data.name,
            data.externalModuleId,
            data.remoteName,
          ];
        }
        return remotesLoadingIdToExternalAndNameMappingMapping;
      },
    );
    early(
      __rspack_require.federation.bundlerRuntimeOptions.remotes,
      'webpackRequire',
      () => __rspack_require,
    );
    merge(
      __rspack_require.federation.bundlerRuntimeOptions.remotes,
      'idToRemoteMap',
      () => {
        const idToRemoteMap = {};
        for (let [id, remoteData] of Object.entries(
          remotesLoadingModuleIdToRemoteDataMapping,
        )) {
          const info =
            __module_federation_remote_infos__[remoteData.remoteName];
          if (info) idToRemoteMap[id] = info;
        }
        return idToRemoteMap;
      },
    );

    override(
      __rspack_require,
      'S',
      __rspack_require.federation.bundlerRuntime.S,
    );
    if (__rspack_require.federation.attachShareScopeMap) {
      __rspack_require.federation.attachShareScopeMap(__rspack_require);
    }

    override(__rspack_require.f, 'remotes', (chunkId, promises) =>
      __rspack_require.federation.bundlerRuntime.remotes({
        chunkId,
        promises,
        chunkMapping: remotesLoadingChunkMapping,
        idToExternalAndNameMapping:
          __rspack_require.federation.bundlerRuntimeOptions.remotes
            .idToExternalAndNameMapping,
        idToRemoteMap:
          __rspack_require.federation.bundlerRuntimeOptions.remotes
            .idToRemoteMap,
        webpackRequire: __rspack_require,
      }),
    );
    override(__rspack_require.f, 'consumes', (chunkId, promises) =>
      __rspack_require.federation.bundlerRuntime.consumes({
        chunkId,
        promises,
        chunkMapping: consumesLoadingChunkMapping,
        moduleToHandlerMapping:
          __rspack_require.federation.consumesLoadingModuleToHandlerMapping,
        installedModules: consumesLoadinginstalledModules,
        webpackRequire: __rspack_require,
      }),
    );
    override(__rspack_require, 'I', (name, initScope) =>
      __rspack_require.federation.bundlerRuntime.I({
        shareScopeName: name,
        initScope,
        initPromises: initializeSharingInitPromises,
        initTokens: initializeSharingInitTokens,
        webpackRequire: __rspack_require,
      }),
    );
    override(
      __rspack_require,
      'initContainer',
      (shareScope, initScope, remoteEntryInitOptions) =>
        __rspack_require.federation.bundlerRuntime.initContainerEntry({
          shareScope,
          initScope,
          remoteEntryInitOptions,
          shareScopeKey: containerShareScope,
          webpackRequire: __rspack_require,
        }),
    );
    override(__rspack_require, 'getContainer', (module, getScope) => {
      var moduleMap = __rspack_require.initializeExposesData.moduleMap;
      __rspack_require.R = getScope;
      getScope = Object.prototype.hasOwnProperty.call(moduleMap, module)
        ? moduleMap[module]()
        : Promise.resolve().then(() => {
            throw new Error(
              'Module "' + module + '" does not exist in container.',
            );
          });
      __rspack_require.R = undefined;
      return getScope;
    });

    __rspack_require.federation.instance =
      __rspack_require.federation.bundlerRuntime.init({
        webpackRequire: __rspack_require,
      });

    if (__rspack_require.consumesLoadingData?.initialConsumes) {
      __rspack_require.federation.bundlerRuntime.installInitialConsumes({
        webpackRequire: __rspack_require,
        installedModules: consumesLoadinginstalledModules,
        initialConsumes: __rspack_require.consumesLoadingData.initialConsumes,
        moduleToHandlerMapping:
          __rspack_require.federation.consumesLoadingModuleToHandlerMapping,
      });
    }
  }
}
