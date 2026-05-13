import { BuiltinPluginName } from '@rspack/binding';

import { create } from './base';

export const ParallelFlagDependencyExportsPlugin = create(
  BuiltinPluginName.ParallelFlagDependencyExportsPlugin,
  () => {},
  'compilation',
);
