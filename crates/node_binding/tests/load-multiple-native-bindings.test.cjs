const assert = require('node:assert/strict');
const { spawnSync } = require('node:child_process');
const {
  copyFileSync,
  existsSync,
  mkdtempSync,
  readFileSync,
  rmSync,
} = require('node:fs');
const { tmpdir } = require('node:os');
const path = require('node:path');
const { test } = require('node:test');

const bindingDir = path.resolve(__dirname, '..');

test(
  'loads the native binding twice from different paths',
  { skip: isWasm() },
  (t) => {
    const nativeBindingPath = findNativeBinding();
    const copyDir = mkdtempSync(path.join(tmpdir(), 'rspack-binding-'));
    const copiedBindingPath = path.join(
      copyDir,
      path.basename(nativeBindingPath),
    );

    t.after(() => {
      try {
        rmSync(copyDir, { recursive: true, force: true });
      } catch (error) {
        if (process.platform !== 'win32') {
          throw error;
        }
      }
    });

    copyFileSync(nativeBindingPath, copiedBindingPath);

    const loadResult = spawnSync(
      process.execPath,
      [
        '-e',
        `
const assert = require("node:assert/strict");
const firstBinding = require(${JSON.stringify(nativeBindingPath)});
const secondBinding = require(${JSON.stringify(copiedBindingPath)});

assert.notStrictEqual(firstBinding, secondBinding);
assert.equal(typeof firstBinding.EXPECTED_RSPACK_CORE_VERSION, "string");
assert.equal(
	secondBinding.EXPECTED_RSPACK_CORE_VERSION,
	firstBinding.EXPECTED_RSPACK_CORE_VERSION
);
`,
      ],
      { encoding: 'utf8' },
    );

    assert.equal(
      loadResult.status,
      0,
      childProcessError(loadResult, nativeBindingPath, copiedBindingPath),
    );
  },
);

function findNativeBinding() {
  const candidates = nativeBindingCandidates().map((fileName) =>
    path.join(bindingDir, fileName),
  );
  const nativeBindingPath = candidates.find(existsSync);

  if (!nativeBindingPath) {
    throw new Error(
      `Cannot find a native binding for ${process.platform}-${process.arch}. Looked for:\n${candidates.join('\n')}`,
    );
  }

  return nativeBindingPath;
}

function isWasm() {
  return process.env.WASM === '1' || process.env.NAPI_RS_FORCE_WASI === '1';
}

function nativeBindingCandidates() {
  switch (process.platform) {
    case 'darwin':
      return [
        'rspack.darwin-universal.node',
        `rspack.darwin-${process.arch}.node`,
      ];
    case 'freebsd':
      return [`rspack.freebsd-${process.arch}.node`];
    case 'linux':
      return linuxBindingCandidates();
    case 'openharmony':
      return [`rspack.linux-${process.arch}-ohos.node`];
    case 'win32':
      return [`rspack.win32-${process.arch}-msvc.node`];
    default:
      throw new Error(`Unsupported platform: ${process.platform}`);
  }
}

function linuxBindingCandidates() {
  if (process.arch === 'arm') {
    return isMusl()
      ? ['rspack.linux-arm-musleabihf.node', 'rspack.linux-arm-gnueabihf.node']
      : ['rspack.linux-arm-gnueabihf.node', 'rspack.linux-arm-musleabihf.node'];
  }

  if (process.arch === 'ppc64' || process.arch === 's390x') {
    return [`rspack.linux-${process.arch}-gnu.node`];
  }

  const libcCandidates = isMusl() ? ['musl', 'gnu'] : ['gnu', 'musl'];
  return libcCandidates.map(
    (libc) => `rspack.linux-${process.arch}-${libc}.node`,
  );
}

function isMusl() {
  let musl = false;

  if (process.platform === 'linux') {
    musl = isMuslFromFilesystem();
    if (musl === null) {
      musl = isMuslFromReport();
    }
    if (musl === null) {
      musl = isMuslFromChildProcess();
    }
  }

  return musl;
}

function isMuslFromFilesystem() {
  try {
    return readFileSync('/usr/bin/ldd', 'utf8').includes('musl');
  } catch {
    return null;
  }
}

function isMuslFromReport() {
  let report = null;
  if (typeof process.report?.getReport === 'function') {
    process.report.excludeNetwork = true;
    report = process.report.getReport();
  }
  if (!report) {
    return null;
  }
  if (report.header && report.header.glibcVersionRuntime) {
    return false;
  }
  if (Array.isArray(report.sharedObjects)) {
    return report.sharedObjects.some(
      (file) => file.includes('libc.musl-') || file.includes('ld-musl-'),
    );
  }
  return false;
}

function isMuslFromChildProcess() {
  try {
    const result = spawnSync('ldd', ['--version'], { encoding: 'utf8' });
    return `${result.stdout ?? ''}${result.stderr ?? ''}`.includes('musl');
  } catch {
    return false;
  }
}

function childProcessError(result, nativeBindingPath, copiedBindingPath) {
  return [
    'Expected the native binding and its copied file to load in one process.',
    `Native binding: ${nativeBindingPath}`,
    `Copied binding: ${copiedBindingPath}`,
    result.signal ? `Signal: ${result.signal}` : `Exit code: ${result.status}`,
    result.stdout ? `stdout:\n${result.stdout}` : '',
    result.stderr ? `stderr:\n${result.stderr}` : '',
  ]
    .filter(Boolean)
    .join('\n');
}
