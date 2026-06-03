import fs from 'node:fs';
import type { Readable } from 'node:stream';
import type {
  MultiStats,
  MultiStatsOptions,
  Stats,
  StatsOptions,
  StatsValue,
} from '@rspack/core';
import type { RspackCLI } from '../cli';
import type { RspackCommand } from '../types';
import {
  type CommonOptionsForBuildAndServe,
  commonOptions,
  commonOptionsForBuildAndServe,
  normalizeCommonOptions,
  setDefaultNodeEnv,
} from '../utils/options';

type BuildOptions = CommonOptionsForBuildAndServe & {
  json?: boolean | string;
};

type StatsFormatOptions = StatsValue | MultiStatsOptions | undefined;

const isMultiStats = (stats: Stats | MultiStats): stats is MultiStats =>
  'stats' in stats;

// Only adjust the StatsFactory result used for CLI text output. The original
// Stats/Compilation data and `--json` output stay unchanged.
const applyCliReportedTime = (stat: Stats, endTime: number) => {
  let enabled = true;
  const { compilation } = stat;
  compilation.hooks.statsFactory.tap('RspackCLIStatsTime', (statsFactory) => {
    statsFactory.hooks.result
      .for('compilation')
      .tap('RspackCLIStatsTime', (statsCompilation, context) => {
        const compilationStats = statsCompilation as { time?: number };
        if (
          enabled &&
          context.compilation === compilation &&
          compilationStats.time !== undefined
        ) {
          compilationStats.time = endTime - stat.startTime;
        }
        return statsCompilation;
      });
  });

  return () => {
    enabled = false;
  };
};

const statsToStringWithEndTime = (
  stats: Stats | MultiStats,
  options: StatsFormatOptions,
  endTime: number,
) => {
  const cleanup: (() => void)[] = [];
  if (isMultiStats(stats)) {
    for (const stat of stats.stats) {
      cleanup.push(applyCliReportedTime(stat, endTime));
    }
  } else {
    cleanup.push(applyCliReportedTime(stats, endTime));
  }

  try {
    return stats.toString(options as StatsOptions);
  } finally {
    for (const fn of cleanup) {
      fn();
    }
  }
};

async function runBuild(cli: RspackCLI, options: BuildOptions): Promise<void> {
  setDefaultNodeEnv(options, 'production');
  normalizeCommonOptions(options, 'build');

  const logger = cli.getLogger();
  let createJsonStringifyStream: ((value: unknown) => Readable) | undefined;

  if (options.json) {
    const stream = await import('node:stream');
    const jsonExt = await import(
      /* webpackChunkName: "json-ext" */ '@discoveryjs/json-ext'
    );
    createJsonStringifyStream = (value) =>
      stream.Readable.from(jsonExt.stringifyChunked(value));
  }

  const errorHandler = (
    error: Error | null,
    stats: Stats | MultiStats | undefined,
    cliReportedEndTime?: number,
  ) => {
    if (error) {
      logger.error(error);
      process.exit(2);
    }

    if (stats?.hasErrors()) {
      process.exitCode = 1;
    }

    if (!compiler || !stats) {
      return;
    }

    const getStatsOptions = () => {
      if (cli.isMultipleCompiler(compiler)) {
        return {
          children: compiler.compilers.map((item) =>
            item.options ? item.options.stats : undefined,
          ),
        } satisfies MultiStatsOptions;
      }
      return compiler.options?.stats;
    };

    const statsOptions = getStatsOptions() as StatsFormatOptions;

    if (options.json && createJsonStringifyStream) {
      const handleWriteError = (error: Error) => {
        logger.error(error);
        process.exit(2);
      };
      if (options.json === true) {
        createJsonStringifyStream(stats.toJson(statsOptions as StatsOptions))
          .on('error', handleWriteError)
          .pipe(process.stdout)
          .on('error', handleWriteError)
          .on('close', () => process.stdout.write('\n'));
      } else if (typeof options.json === 'string') {
        createJsonStringifyStream(stats.toJson(statsOptions as StatsOptions))
          .on('error', handleWriteError)
          .pipe(fs.createWriteStream(options.json))
          .on('error', handleWriteError)
          // Use stderr to logging
          .on('close', () => {
            process.stderr.write(
              `[rspack-cli] ${cli.colors.green(
                `stats are successfully stored as json to ${options.json}`,
              )}\n`,
            );
          });
      }
    } else {
      const printedStats =
        cliReportedEndTime === undefined
          ? stats.toString(statsOptions as StatsOptions)
          : statsToStringWithEndTime(stats, statsOptions, cliReportedEndTime);
      // Avoid extra empty line when `stats: 'none'`
      if (printedStats) {
        logger.raw(printedStats);
      }
    }
  };

  const userOption = await cli.buildCompilerConfig(options, 'build');
  const compiler = await cli.createCompiler(userOption, errorHandler);

  if (!compiler || cli.isWatch(compiler)) {
    return;
  }

  compiler.run((error: Error | null, stats: Stats | MultiStats | undefined) => {
    compiler.close((closeErr) => {
      const cliReportedEndTime = Date.now();
      if (closeErr) {
        logger.error(closeErr);
      }
      errorHandler(error, stats, cliReportedEndTime);
    });
  });
}

export class BuildCommand implements RspackCommand {
  apply(cli: RspackCLI): void {
    const command = cli.program
      .command('', 'run the Rspack build')
      .alias('build')
      .alias('bundle')
      .alias('b');

    commonOptionsForBuildAndServe(commonOptions(command)).option(
      '--json [path]',
      'emit stats json',
    );

    command.action(
      cli.wrapAction(async (options: BuildOptions) => {
        await runBuild(cli, options);
      }),
    );
  }
}
