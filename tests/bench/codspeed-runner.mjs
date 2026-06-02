import { performance } from 'node:perf_hooks';
import process from 'node:process';
import {
  getInstrumentMode,
  InstrumentHooks,
  setupCore,
  teardownCore,
} from '@codspeed/core';

const rootBeforeAll = [];
const suites = [];
let currentSuite = null;

function getCurrentBeforeAllList() {
  return currentSuite ? currentSuite.beforeAll : rootBeforeAll;
}

export function beforeAll(fn) {
  getCurrentBeforeAllList().push(fn);
}

export function describe(name, fn) {
  const suite = {
    name,
    beforeAll: [],
    benchmarks: [],
    suites: [],
  };

  if (currentSuite) {
    currentSuite.suites.push(suite);
  } else {
    suites.push(suite);
  }

  const previousSuite = currentSuite;
  currentSuite = suite;
  try {
    fn();
  } finally {
    currentSuite = previousSuite;
  }
}

export function bench(name, fn, options = {}) {
  if (!currentSuite) {
    throw new Error(
      `bench(${JSON.stringify(name)}) must be declared inside describe()`,
    );
  }

  currentSuite.benchmarks.push({
    name: name.startsWith('js@') ? name : `js@${name}`,
    fn,
    options,
  });
}

async function runHook(fn) {
  await fn();
}

async function runWarmup({ fn, options }) {
  const iterations = Number.parseInt(
    process.env.RSPACK_JS_BENCH_WARMUP_ITERATIONS ??
      `${options.warmupIterations ?? 1}`,
    10,
  );

  for (let i = 0; i < iterations; i++) {
    await fn();
  }
}

async function runUninstrumented(uri, fn) {
  const start = performance.now();
  await fn();
  const duration = performance.now() - start;
  console.log(`[bench] ${uri} ${duration.toFixed(3)}ms`);
}

async function runInstrumented(uri, fn) {
  await (async function __codspeed_root_frame__() {
    const startResult = InstrumentHooks.startBenchmark();
    if (startResult !== 0) {
      throw new Error(
        `CodSpeed startBenchmark failed with code ${startResult}`,
      );
    }

    try {
      await fn();
    } finally {
      const stopResult = InstrumentHooks.stopBenchmark();
      const setResult = InstrumentHooks.setExecutedBenchmark(process.pid, uri);
      if (stopResult !== 0) {
        throw new Error(
          `CodSpeed stopBenchmark failed with code ${stopResult}`,
        );
      }
      if (setResult !== 0) {
        throw new Error(
          `CodSpeed setExecutedBenchmark failed with code ${setResult}`,
        );
      }
    }
  })();
}

async function runBenchmark(benchmark, suitePath, instrumentMode) {
  const uri = `${suitePath}::${benchmark.name}`;

  await runWarmup(benchmark);
  global.gc?.();

  if (instrumentMode === 'analysis' || instrumentMode === 'memory') {
    await runInstrumented(uri, benchmark.fn);
    console.log(`[CodSpeed] ${uri} done`);
  } else {
    await runUninstrumented(uri, benchmark.fn);
  }
}

async function runSuite(suite, parentPath, instrumentMode) {
  const suitePath = parentPath ? `${parentPath}::${suite.name}` : suite.name;

  for (const hook of suite.beforeAll) {
    await runHook(hook);
  }

  for (const benchmark of suite.benchmarks) {
    await runBenchmark(benchmark, suitePath, instrumentMode);
  }

  for (const childSuite of suite.suites) {
    await runSuite(childSuite, suitePath, instrumentMode);
  }
}

export async function runBenchmarks({ baseUri }) {
  const instrumentMode = getInstrumentMode();
  const codspeedEnabled = instrumentMode !== 'disabled';

  if (codspeedEnabled) {
    if (instrumentMode === 'analysis' && !InstrumentHooks.isInstrumented()) {
      throw new Error('[CodSpeed] bench detected but no instrumentation found');
    }
    setupCore();
  } else {
    console.log(
      '[bench] CodSpeed is disabled; running once with local wall-clock timings.',
    );
  }

  try {
    for (const hook of rootBeforeAll) {
      await runHook(hook);
    }

    for (const suite of suites) {
      await runSuite(suite, baseUri, instrumentMode);
    }
  } finally {
    if (codspeedEnabled) {
      teardownCore();
    }
  }
}
