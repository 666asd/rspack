// @ts-nocheck

/* istanbul ignore file */

import fs from 'node:fs';
import path from 'node:path';
import chalk from 'chalk';
import filenamify from 'filenamify';
import { diff } from 'jest-diff';
import { serializers } from '../serializers';
import {
  getSnapshotSerializers,
  serializeSnapshot,
} from './snapshot-serializers';

/**
 * Check if 2 strings or buffer are equal
 */
const isEqual = (a: string | Buffer, b: string | Buffer): boolean => {
  // @ts-expect-error: TypeScript gives error if we pass string to buffer.equals
  return Buffer.isBuffer(a) ? a.equals(b) : a === b;
};

function readSnapshot(filename: string, content: string | Buffer) {
  const output = fs.readFileSync(
    filename,
    Buffer.isBuffer(content) ? null : 'utf8',
  );
  return Buffer.isBuffer(output) ? output : output.replace(/\r\n/g, '\n');
}

function toPosixPath(filename: string) {
  return filename.split(path.sep).join('/');
}

function getRuntimeProxyBaseSnapshotPath(filename: string): string | undefined {
  if (!process.env.RSPACK_TEST_RUNTIME_REQUIREMENTS_PROXY) {
    return;
  }

  const normalized = toPosixPath(filename);
  const base = normalized
    .replace('/__runtime_proxy_snapshot__/', '/__snapshot__/')
    .replace('/__runtime_proxy_snapshots__/', '/__snapshots__/');

  if (base === normalized) {
    return;
  }

  return path.normalize(base);
}

function getRuntimeProxySnapshotRoot(filename: string): string | undefined {
  const normalized = toPosixPath(filename);
  const markers = [
    '/__runtime_proxy_snapshot__/',
    '/__runtime_proxy_snapshots__/',
  ];
  for (const marker of markers) {
    const markerIndex = normalized.indexOf(marker);
    if (markerIndex >= 0) {
      return path.normalize(
        normalized.slice(0, markerIndex + marker.length - 1),
      );
    }
  }
}

function removeEmptyParentsUntil(dir: string, stopDir: string) {
  let current = dir;
  while (current.startsWith(stopDir) && current !== stopDir) {
    try {
      fs.rmdirSync(current);
    } catch {
      break;
    }
    current = path.dirname(current);
  }
}

/**
 * Match given content against content of the specified file.
 *
 * @param content Output content to match
 * @param filepath Path to the file to match against
 * @param options Additional options for matching
 */
export function toMatchFileSnapshotSync(
  this: {
    testPath: string;
    currentTestName: string;
    assertionCalls: number;
    isNot: boolean;
    snapshotState: {
      added: number;
      updated: number;
      unmatched: number;
      _updateSnapshot: 'none' | 'new' | 'all';
    };
  },
  rawContent: string | Buffer,
  filepath: string,
  options: FileMatcherOptions = {},
) {
  const content = Buffer.isBuffer(rawContent)
    ? rawContent
    : serializeSnapshot(rawContent, /* ident */ 2, {
        plugins: [
          ...getSnapshotSerializers(),
          // Rspack serializers
          ...serializers,
        ],
      });

  const { isNot, snapshotState } = this;

  const filename =
    filepath === undefined
      ? // If file name is not specified, generate one from the test title
        path.join(
          path.dirname(this.testPath),
          '__file_snapshots__',
          `${filenamify(this.currentTestName, {
            replacement: '-',
          }).replace(/\s/g, '-')}-${this.assertionCalls}`,
        )
      : filepath;

  const baseSnapshotFilename = getRuntimeProxyBaseSnapshotPath(filename);
  const baseSnapshotExists =
    baseSnapshotFilename !== undefined && fs.existsSync(baseSnapshotFilename);
  const matchedFilename = fs.existsSync(filename)
    ? filename
    : baseSnapshotExists
      ? baseSnapshotFilename
      : filename;

  if (
    snapshotState._updateSnapshot === 'none' &&
    !fs.existsSync(matchedFilename)
  ) {
    // We're probably running in CI environment

    snapshotState.unmatched++;

    return {
      pass: isNot,
      message: () =>
        `New output file ${chalk.blue(
          path.basename(filename),
        )} was ${chalk.bold.red('not written')}.\n\nThe update flag must be explicitly passed to write a new snapshot.\n\nThis is likely because this test is run in a ${chalk.blue(
          'continuous integration (CI) environment',
        )} in which snapshots are not written by default.\n\n`,
    };
  }

  if (fs.existsSync(matchedFilename)) {
    const output = readSnapshot(matchedFilename, content);

    if (isNot) {
      // The matcher is being used with `.not`

      if (!isEqual(content, output)) {
        // The value of `pass` is reversed when used with `.not`
        return { pass: false, message: () => '' };
      }
      snapshotState.unmatched++;

      return {
        pass: true,
        message: () =>
          `Expected received content ${chalk.red(
            'to not match',
          )} the file ${chalk.blue(path.basename(filename))}.`,
      };
    }
    if (isEqual(content, output)) {
      return { pass: true, message: () => '' };
    }
    if (snapshotState._updateSnapshot === 'all') {
      if (baseSnapshotExists) {
        const baseOutput = readSnapshot(baseSnapshotFilename, content);
        if (isEqual(content, baseOutput)) {
          if (fs.existsSync(filename)) {
            fs.unlinkSync(filename);
            const runtimeProxySnapshotRoot =
              getRuntimeProxySnapshotRoot(filename);
            if (runtimeProxySnapshotRoot) {
              removeEmptyParentsUntil(
                path.dirname(filename),
                runtimeProxySnapshotRoot,
              );
            }
          }

          snapshotState.updated++;

          return { pass: true, message: () => '' };
        }
      }
      fs.mkdirSync(path.dirname(filename), { recursive: true });
      fs.writeFileSync(filename, content);

      snapshotState.updated++;

      return { pass: true, message: () => '' };
    }
    snapshotState.unmatched++;

    const difference =
      Buffer.isBuffer(content) || Buffer.isBuffer(output)
        ? ''
        : `\n\n${diff(
            output,
            content,
            Object.assign(
              {
                expand: false,
                contextLines: 5,
                aAnnotation: 'Snapshot',
              },
              options.diff || {},
            ),
          )}`;

    return {
      pass: false,
      message: () =>
        `Received content ${chalk.red(
          "doesn't match",
        )} the file ${chalk.blue(path.basename(filename))}.${difference}`,
    };
  }
  if (
    !isNot &&
    (snapshotState._updateSnapshot === 'new' ||
      snapshotState._updateSnapshot === 'all')
  ) {
    if (baseSnapshotExists) {
      const baseOutput = readSnapshot(baseSnapshotFilename, content);
      if (isEqual(content, baseOutput)) {
        return { pass: true, message: () => '' };
      }
    }
    fs.mkdirSync(path.dirname(filename), { recursive: true });
    fs.writeFileSync(filename, content);

    snapshotState.added++;

    return { pass: true, message: () => '' };
  }
  snapshotState.unmatched++;

  return {
    pass: true,
    message: () =>
      `The output file ${chalk.blue(
        path.basename(filename),
      )} ${chalk.bold.red("doesn't exist")}.`,
  };
}
