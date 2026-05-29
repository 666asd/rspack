import { type BuiltinPlugin, BuiltinPluginName } from '@rspack/binding';
import type { Module } from '../Module';
import { createBuiltinPlugin, RspackBuiltinPlugin } from './base';

export interface SyncModuleIdsPluginOptions {
  path: string;
  context?: string;
  test?: (module: Module) => boolean;
  mode?: 'read' | 'create' | 'merge' | 'update';
}

export class SyncModuleIdsPlugin extends RspackBuiltinPlugin {
  name = BuiltinPluginName.SyncModuleIdsPlugin;
  affectedHooks = 'compilation' as const;

  constructor(private options: SyncModuleIdsPluginOptions) {
    super();
  }

  raw(): BuiltinPlugin {
    return createBuiltinPlugin(this.name, { ...this.options });
  }
}
