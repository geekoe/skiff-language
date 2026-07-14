#!/usr/bin/env node

import path from 'node:path';

import {
  collectLoopRiskUrlArgs,
  formatLoopRiskJson,
  isMainModule,
  parseLoopRiskArgs,
  readNonNegativeIntegerArg,
  readNumberArg,
  readPositiveIntegerArg,
} from './lib/loop-risk-cli.mjs';
import {
  LOOP_RISK_CONFIG_PROFILES,
  loadLoopRiskConfig,
} from './lib/loop-risk-config.mjs';
import { runLoopRiskStress } from './lib/loop-risk-stress.mjs';
import {
  createNodeLoopRiskStressAdapters,
  loadRouterWebSocket,
  resolveRuntimePidsFromPgrep,
} from './lib/loop-risk-stress-node.mjs';

const argv = process.argv.slice(2);
const knownRawUrls = unique([
  ...collectLoopRiskUrlArgs(argv, ['ws-url', 'health-url']),
  process.env.SKIFF_LOOP_RISK_WS_URL,
].filter(Boolean));

if (isMainModule(import.meta.url)) {
  main().catch((error) => {
    console.error(formatLoopRiskJson({
      ok: false,
      message: error instanceof Error ? error.message : String(error),
    }, knownRawUrls));
    process.exitCode = 1;
  });
}

async function main() {
  const args = parseStressArgs(argv);
  if (args.hasFlag('help')) {
    printUsage();
    return;
  }

  const config = await resolveStressConfig(args);
  knownRawUrls.push(config.wsUrl, config.healthUrl);
  const WebSocket = await loadRouterWebSocket(import.meta.url);
  const result = await runLoopRiskStress(
    config,
    createNodeLoopRiskStressAdapters(WebSocket),
  );
  if (config.canonical && [
    result.health,
    result.cpu,
    result.runtimeRequestErrorLogs,
  ].some((summary) => summary.checked !== true)) {
    throw new Error('canonical loop-risk stress completed with an unchecked gate');
  }
  console.log(formatLoopRiskJson(result, knownRawUrls));
}

function parseStressArgs(rawArgs) {
  return parseLoopRiskArgs(rawArgs, {
    flags: ['help', 'skip-health', 'skip-cpu', 'skip-log-check'],
    singletonValues: [
      'config',
      'ws-url',
      'health-url',
      'messages',
      'concurrency',
      'health-timeout-ms',
      'session-prefix',
      'payload',
      'open-timeout-ms',
      'close-timeout-ms',
      'close-delay-ms',
      'max-new-runtime-request-errors',
      'runtime-pgrep',
      'cpu-seconds',
      'cpu-interval-ms',
      'cpu-median-threshold',
      'cpu-post-grace-threshold',
      'cpu-grace-seconds',
    ],
    repeatableValues: [
      'header',
      'runtime-id',
      'runtime-ids',
      'runtime-log',
      'log-file',
      'runtime-pid',
      'runtime-pids',
    ],
  });
}

async function resolveStressConfig(args) {
  const common = readCommonConfig(args);
  const configPath = args.value('config');
  if (configPath !== undefined) {
    assertCanonicalArguments(args);
    const canonical = await loadLoopRiskConfig(path.resolve(configPath), {
      profile: LOOP_RISK_CONFIG_PROFILES.STRESS,
      checkLogFiles: true,
    });
    return {
      ...common,
      canonical: true,
      wsUrl: canonical.stress.wsUrl,
      healthUrl: canonical.healthUrl,
      runtimeIds: canonical.runtimeIds,
      runtimePids: canonical.stress.runtimePids,
      runtimeLogs: canonical.stress.runtimeLogs,
      skipHealth: false,
      skipCpu: false,
      skipLogCheck: false,
    };
  }

  const wsUrl = args.value('ws-url') ?? process.env.SKIFF_LOOP_RISK_WS_URL;
  if (!wsUrl) {
    throw new Error('--ws-url or SKIFF_LOOP_RISK_WS_URL is required');
  }
  const skipHealth = args.hasFlag('skip-health');
  const skipCpu = args.hasFlag('skip-cpu');
  const skipLogCheck = args.hasFlag('skip-log-check');
  const healthUrl = args.value('health-url');
  const runtimeIds = unique(args.list('runtime-id', 'runtime-ids'));
  const runtimeLogs = args.list('runtime-log', 'log-file');
  const explicitRuntimePids = parseRuntimePids(args);
  const runtimePgrep = args.value('runtime-pgrep');

  if (!skipHealth && (!healthUrl || runtimeIds.length === 0)) {
    throw new Error(
      '--health-url and --runtime-id(s) are required unless --skip-health is explicit',
    );
  }
  if (!skipLogCheck && runtimeLogs.length === 0) {
    throw new Error('--runtime-log or --log-file is required unless --skip-log-check is explicit');
  }
  if (!skipCpu && explicitRuntimePids.length === 0 && !runtimePgrep) {
    throw new Error('--runtime-pid or --runtime-pgrep is required unless --skip-cpu is explicit');
  }
  if (explicitRuntimePids.length > 0 && runtimePgrep) {
    throw new Error('--runtime-pid and --runtime-pgrep are mutually exclusive');
  }
  const runtimePids = skipCpu
    ? explicitRuntimePids
    : await resolveRuntimePidsFromPgrep(explicitRuntimePids, runtimePgrep);
  return {
    ...common,
    canonical: false,
    wsUrl,
    healthUrl,
    runtimeIds,
    runtimePids,
    runtimeLogs,
    skipHealth,
    skipCpu,
    skipLogCheck,
  };
}

function readCommonConfig(args) {
  return {
    messages: readPositiveIntegerArg(args, 'messages', 1000),
    concurrency: readPositiveIntegerArg(args, 'concurrency', 50),
    healthTimeoutMs: readPositiveIntegerArg(args, 'health-timeout-ms', 5000),
    healthIntervalMs: 250,
    headers: parseHeaders(args),
    sessionPrefix: args.value('session-prefix') ?? `loop-risk-stress-${Date.now()}`,
    payloadTemplate:
      args.value('payload') ?? '{"tag":"loop_risk_ws_cancel_stress","index":{index}}',
    openTimeoutMs: readPositiveIntegerArg(args, 'open-timeout-ms', 5000),
    closeTimeoutMs: readPositiveIntegerArg(args, 'close-timeout-ms', 5000),
    closeDelayMs: readNonNegativeIntegerArg(args, 'close-delay-ms', 0),
    maxNewRuntimeRequestErrors: readNonNegativeIntegerArg(
      args,
      'max-new-runtime-request-errors',
      0,
    ),
    cpu: {
      seconds: readPositiveIntegerArg(args, 'cpu-seconds', 30),
      intervalMs: readPositiveIntegerArg(args, 'cpu-interval-ms', 1000),
      medianThreshold: readNumberArg(args, 'cpu-median-threshold', 5),
      postGraceThreshold: readNumberArg(args, 'cpu-post-grace-threshold', 25),
      graceSeconds: readNonNegativeIntegerArg(args, 'cpu-grace-seconds', 10),
    },
  };
}

function assertCanonicalArguments(args) {
  const forbiddenValues = [
    'ws-url',
    'health-url',
    'runtime-id',
    'runtime-ids',
    'runtime-log',
    'log-file',
    'runtime-pid',
    'runtime-pids',
    'runtime-pgrep',
  ];
  const supplied = forbiddenValues.filter((name) => args.values(name).length > 0);
  const skipFlags = ['skip-health', 'skip-cpu', 'skip-log-check']
    .filter((name) => args.hasFlag(name));
  if (process.env.SKIFF_LOOP_RISK_WS_URL) {
    supplied.push('SKIFF_LOOP_RISK_WS_URL');
  }
  if (supplied.length > 0 || skipFlags.length > 0) {
    throw new Error(
      `--config cannot be combined with target overrides or skip flags: ${[
        ...supplied.map((name) => name.startsWith('SKIFF_') ? name : `--${name}`),
        ...skipFlags.map((name) => `--${name}`),
      ].join(', ')}`,
    );
  }
}

function parseRuntimePids(args) {
  return unique(args.list('runtime-pid', 'runtime-pids').map((rawPid) => {
    const pid = Number(rawPid);
    if (!Number.isInteger(pid) || pid <= 0) {
      throw new Error(`--runtime-pid must contain only positive integers; got ${rawPid}`);
    }
    return pid;
  }));
}

function parseHeaders(args) {
  const headers = {};
  for (const entry of args.values('header')) {
    const separator = entry.indexOf('=');
    if (separator <= 0) {
      throw new Error(`--header must be name=value, got ${entry}`);
    }
    const name = entry.slice(0, separator).trim().toLowerCase();
    if (!name || Object.hasOwn(headers, name)) {
      throw new Error(`--header name must be non-empty and unique: ${name || '<empty>'}`);
    }
    headers[name] = entry.slice(separator + 1).trim();
  }
  return headers;
}

function unique(values) {
  return Array.from(new Set(values));
}

function printUsage() {
  console.log(`Usage:
  node scripts/check-loop-risk-stress-live.mjs --config <path>
  node scripts/check-loop-risk-stress-live.mjs --ws-url <url> [direct options]

Canonical:
  --config <path>                 Strict config; health/CPU/log checks cannot be skipped.

Direct targets:
  --ws-url <url>                  Explicit websocket URL.
  --health-url <url>              Required with --runtime-id unless --skip-health.
  --runtime-id <id>               May be repeated or comma-separated.
  --runtime-pid <pid>             May be repeated or comma-separated.
  --runtime-pgrep <pattern>       Explicit diagnostic pgrep alternative.
  --runtime-log <file>            Required unless --skip-log-check.

Stress tuning:
  --messages <n>                  Default: 1000.
  --concurrency <n>               Default: 50.
  --payload <text>                "{index}" is replaced per attempt.
  --header name=value             May be repeated; values may contain commas.
  --health-timeout-ms <ms>        Default: 5000.
  --cpu-seconds <n>               Default: 30 samples.
  --cpu-median-threshold <pct>    Default: 5.
  --cpu-post-grace-threshold <pct> Default: 25.
  --max-new-runtime-request-errors <n> Default: 0.

Explicit direct-only skips:
  --skip-health
  --skip-cpu
  --skip-log-check`);
}
