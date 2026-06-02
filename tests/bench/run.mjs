import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { getGitDir, tryIntrospect } from '@codspeed/core';
import { runBenchmarks } from './codspeed-runner.mjs';

tryIntrospect();

const benchmarkUrl = new URL('./ts-react.bench.mjs', import.meta.url);
const benchmarkPath = fileURLToPath(benchmarkUrl);
const gitDir = getGitDir(benchmarkPath) ?? process.cwd();
const baseUri = path.relative(gitDir, benchmarkPath);

await import(benchmarkUrl.href);
await runBenchmarks({ baseUri });
