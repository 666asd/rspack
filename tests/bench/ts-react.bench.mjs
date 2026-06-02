import path from 'node:path';
import { NormalModule, rspack } from '@rspack/core';
import { beforeAll, bench, describe } from './codspeed-runner.mjs';
import rspackConfig from './fixtures/ts-react/rspack.config.mjs';

const BARREL_OPTIMIZATION_PREFIX = '__barrel_optimize__';

let context;
let theCompilation;
let externalContexts = [];
let blackhole = 0;

function consume(value) {
  if (value == null) {
    return;
  }
  blackhole ^= String(value).length;
}

beforeAll(() => {
  externalContexts = [];
  return new Promise((resolve, reject) =>
    rspack(
      {
        ...rspackConfig,
        mode: 'production',
        plugins: [
          ...(rspackConfig.plugins ?? []),
          (compiler) => {
            context = compiler.context;

            compiler.hooks.compilation.tap('PLUGIN', (compilation) => {
              theCompilation = compilation;
            });
          },
        ],
        externals: [
          (context, callback) => {
            externalContexts.push(context);
            callback();
          },
        ],
      },
      (err, stats) => {
        if (err) {
          reject(err);
          return;
        }
        if (stats?.hasErrors()) {
          reject(new Error(stats.toString({})));
          return;
        }
        resolve(undefined);
      },
    ),
  );
});

describe('TypeScript React project', () => {
  bench('Traverse module graph by dependencies', () => {
    const entries = theCompilation.entries.values();

    const visitedModules = new Set();

    function traverse(dependency) {
      const module = theCompilation.moduleGraph.getModule(dependency);
      if (module) {
        if (visitedModules.has(module)) {
          return;
        }
        visitedModules.add(module);
        for (const dep of module.dependencies) {
          traverse(dep);
        }
      }
    }

    for (const entry of entries) {
      for (const dependency of entry.dependencies) {
        traverse(dependency);
      }
    }
  });

  bench('Traverse module graph by connections', () => {
    const entries = theCompilation.entries.values();

    const visitedModules = new Set();

    function traverse(connection) {
      const module = connection ? connection.module : null;
      if (module) {
        if (visitedModules.has(module)) {
          return;
        }
        const connections =
          theCompilation.moduleGraph.getOutgoingConnections(module);
        visitedModules.add(module);
        for (const c of connections) {
          traverse(c);
        }
      }
    }

    for (const entry of entries) {
      for (const dependency of entry.dependencies) {
        const connection = theCompilation.moduleGraph.getConnection(dependency);
        traverse(connection);
      }
    }
  });

  bench('Traverse compilation.modules', () => {
    for (const module of theCompilation.modules) {
      module.identifier();
    }
  });

  bench('stats.toJson()', () => {
    const json = theCompilation.getStats().toJson({
      all: true,
    });
    consume(json.hash);
  });

  bench('collect imported identifiers', () => {
    for (const [, entry] of theCompilation.entries.entries()) {
      const entryDependency = entry.dependencies?.[0];
      if (!entryDependency || !entryDependency.request) continue;

      const entryModule =
        theCompilation.moduleGraph.getResolvedModule(entryDependency);
      if (!entryModule) continue;

      for (const connection of theCompilation.moduleGraph.getOutgoingConnectionsInOrder(
        entryModule,
      )) {
        let importedIdentifiers = [];
        if (connection.dependency?.ids) {
          importedIdentifiers.push(...connection.dependency.ids);
        } else {
          importedIdentifiers = ['*'];
        }
        consume(importedIdentifiers.length);
      }
    }
  });

  bench('record module', () => {
    function recordModule(mod) {
      const resource =
        mod.type === 'css/mini-extract'
          ? mod.identifier().slice(mod.identifier().lastIndexOf('!') + 1)
          : mod.resource;

      if (!resource) {
        return;
      }

      consume(
        path.relative(context, mod.resourceResolveData?.path || resource),
      );
      consume(
        path.relative(context, mod.resourceResolveData?.path || resource),
      );

      const esmResource = /[\\/]next[\\/]dist[\\/]/.test(resource)
        ? resource.replace(
            /[\\/]next[\\/]dist[\\/]/,
            '/next/dist/esm/'.replace(/\//g, path.sep),
          )
        : null;
      consume(esmResource);

      consume(mod.matchResource?.startsWith(BARREL_OPTIMIZATION_PREFIX));
    }

    for (const module of theCompilation.modules) {
      if (module instanceof NormalModule) {
        recordModule(module);
      }
    }
  });

  bench('is css mod', () => {
    const regexCSS = /\.(css|scss|sass)(\?.*)?$/;

    function isCSSMod(mod) {
      return !!(
        mod.type === 'css/mini-extract' ||
        (mod.resource && regexCSS.test(mod.resource)) ||
        mod.loaders?.some(
          ({ loader }) =>
            loader.includes('next-style-loader/index.js') ||
            loader.includes('rspack.CssExtractRspackPlugin.loader') ||
            loader.includes('@vanilla-extract/webpack-plugin/loader/'),
        )
      );
    }

    for (const module of theCompilation.modules) {
      if (module instanceof NormalModule) {
        consume(isCSSMod(module));
      }
    }
  });

  bench('record chunk group', () => {
    const checkedChunkGroups = new Set();
    const checkedChunks = new Set();

    for (const [, entrypoint] of theCompilation.entrypoints) {
      recordChunkGroup(entrypoint);
    }

    function recordChunkGroup(chunkGroup) {
      if (checkedChunkGroups.has(chunkGroup)) return;
      checkedChunkGroups.add(chunkGroup);

      chunkGroup.chunks.forEach((chunk) => {
        if (checkedChunks.has(chunk)) return;
        checkedChunks.add(chunk);
        const entryMods =
          theCompilation.chunkGraph.getChunkEntryModulesIterable(chunk);

        for (const mod of entryMods) {
          for (const connection of theCompilation.moduleGraph.getOutgoingConnectionsInOrder(
            mod,
          )) {
            const dependency = connection.dependency;
            if (!dependency) continue;
            const clientEntryMod =
              theCompilation.moduleGraph.getResolvedModule(dependency);
            consume(theCompilation.chunkGraph.getModuleId(clientEntryMod));
          }
        }
      });

      // Walk through all children chunk groups too.
      for (const child of chunkGroup.childrenIterable) {
        recordChunkGroup(child);
      }
    }
  });

  bench('external getResolve', async () => {
    const values = [];
    for (const { context, request, getResolve } of externalContexts) {
      const resolve = getResolve();
      const result = resolve(context, request);
      values.push(result);
    }
    await Promise.all(values);
  });
});
