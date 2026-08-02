#!/usr/bin/env node

import assert from 'node:assert/strict';
import { resolve } from 'node:path';

import {
  collectLoopRiskUrlArgs,
  formatLoopRiskJson,
  isMainModule,
  parseLoopRiskArgs,
  readPositiveIntegerArg,
} from './lib/loop-risk-cli.mjs';
import {
  LOOP_RISK_CONFIG_PROFILES,
  loadLoopRiskConfig,
} from './lib/loop-risk-config.mjs';
import {
  evaluateLoopRiskHealth,
  pollLoopRiskHealth,
} from './lib/loop-risk-health.mjs';

const argv = process.argv.slice(2);
const knownRawUrls = collectLoopRiskUrlArgs(argv, ['url']);

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
  const args = parseLoopRiskArgs(argv, {
    flags: ['help', 'self-test'],
    singletonValues: ['config', 'url', 'timeout-ms', 'interval-ms'],
    repeatableValues: ['runtime-id', 'runtime-ids'],
  });
  if (args.hasFlag('help')) {
    printUsage();
    return;
  }
  if (args.hasFlag('self-test')) {
    runSelfTest();
    return;
  }

  const target = await resolveTarget(args);
  knownRawUrls.push(target.url);
  const timeoutMs = readPositiveIntegerArg(args, 'timeout-ms', 5000);
  const intervalMs = readPositiveIntegerArg(args, 'interval-ms', 250);
  const result = await pollLoopRiskHealth({
    url: target.url,
    touchedRuntimeIds: target.runtimeIds,
    timeoutMs,
    intervalMs,
  });
  const output = {
    ...result,
    checked: result.ok,
    url: target.url,
    touchedRuntimeIds: target.runtimeIds,
  };
  const rendered = formatLoopRiskJson(output, knownRawUrls);
  if (result.ok) {
    console.log(rendered);
    return;
  }
  console.error(rendered);
  process.exitCode = 1;
}

async function resolveTarget(args) {
  const configPath = args.value('config');
  const runtimeIds = unique(args.list('runtime-id', 'runtime-ids'));
  if (configPath !== undefined) {
    if (args.value('url') !== undefined || runtimeIds.length > 0) {
      throw new Error('--config cannot be combined with --url or --runtime-id(s)');
    }
    const config = await loadLoopRiskConfig(resolve(configPath), {
      profile: LOOP_RISK_CONFIG_PROFILES.HEALTH,
      checkLogFiles: false,
    });
    return { url: config.healthUrl, runtimeIds: config.runtimeIds };
  }
  const url = args.value('url');
  if (!url) {
    throw new Error('--url is required unless --config is provided');
  }
  return { url, runtimeIds };
}

function runSelfTest() {
  const zeroCounters = {
    outboundRequestsPending: 0,
    outboundStreamLeasesActive: 0,
    streamRuntimeStreamsActive: 0,
    flagBackedCancelWaitersActive: 0,
    spawnedTasksActive: 0,
  };
  const zeroRouter = {
    dispatcher: { pendingUnary: 0, pendingStream: 0 },
    httpStream: { backpressureWaiters: 0, backpressureCancels: 0 },
  };
  const connectedZero = {
    runtimeId: 'runtime-a',
    connected: true,
    fresh: true,
    counters: zeroCounters,
  };
  const healthy = {
    router: zeroRouter,
    runtimes: [connectedZero],
  };
  assert.equal(
    evaluateLoopRiskHealth(healthy, { touchedRuntimeIds: ['runtime-a'] }).ok,
    true,
  );
  assert.equal(
    evaluateLoopRiskHealth(healthy, { touchedRuntimeIds: ['runtime-missing'] }).ok,
    false,
  );
  // The canonical AssemblyControlPlane projection retains disconnected replica
  // records with their last health counters. Zero disconnected sessions must
  // stay acceptable, while disconnected nonzero leaks must keep failing.
  const withDisconnectedZero = {
    router: zeroRouter,
    runtimes: [
      connectedZero,
      {
        runtimeId: 'runtime-disconnected',
        connected: false,
        fresh: false,
        counters: zeroCounters,
      },
    ],
  };
  assert.equal(
    evaluateLoopRiskHealth(withDisconnectedZero, { touchedRuntimeIds: [] }).ok,
    true,
  );
  const withDisconnectedNonzero = {
    router: zeroRouter,
    runtimes: [
      connectedZero,
      {
        runtimeId: 'runtime-disconnected',
        connected: false,
        fresh: false,
        counters: { ...zeroCounters, outboundRequestsPending: 1 },
      },
    ],
  };
  assert.equal(
    evaluateLoopRiskHealth(withDisconnectedNonzero, { touchedRuntimeIds: [] }).ok,
    false,
  );

  // Canonical full shape produced by the Rust /__router/health projection
  // (batch 12 health leaf): observedAt plus the TS AssemblyControlPlane
  // loopRisk fields must evaluate clean with all-zero counters.
  const fullCanonical = {
    observedAt: '2026-08-03T00:00:00.000Z',
    router: zeroRouter,
    runtimes: [connectedZero],
  };
  assert.equal(
    evaluateLoopRiskHealth(fullCanonical, { touchedRuntimeIds: ['runtime-a'] }).ok,
    true,
    'full canonical Rust loopRisk shape must be accepted',
  );

  // Missing required evaluator fields must keep failing: each router counter
  // and each runtime counter is independently required.
  const missingHttpStreamWaiters = {
    router: {
      dispatcher: { pendingUnary: 0, pendingStream: 0 },
      httpStream: { backpressureCancels: 0 },
    },
    runtimes: [connectedZero],
  };
  assert.equal(
    evaluateLoopRiskHealth(missingHttpStreamWaiters, { touchedRuntimeIds: [] }).ok,
    false,
    'missing httpStream.backpressureWaiters must fail',
  );
  const missingDispatcherStream = {
    router: {
      dispatcher: { pendingUnary: 0 },
      httpStream: { backpressureWaiters: 0, backpressureCancels: 0 },
    },
    runtimes: [connectedZero],
  };
  assert.equal(
    evaluateLoopRiskHealth(missingDispatcherStream, { touchedRuntimeIds: [] }).ok,
    false,
    'missing dispatcher.pendingStream must fail',
  );
  const missingRuntimeCounter = {
    router: zeroRouter,
    runtimes: [{
      runtimeId: 'runtime-a',
      connected: true,
      fresh: true,
      counters: {
        outboundRequestsPending: 0,
        outboundStreamLeasesActive: 0,
        streamRuntimeStreamsActive: 0,
        flagBackedCancelWaitersActive: 0,
      },
    }],
  };
  assert.equal(
    evaluateLoopRiskHealth(missingRuntimeCounter, { touchedRuntimeIds: [] }).ok,
    false,
    'missing runtime counter must fail',
  );
  console.log(JSON.stringify({ ok: true, selfTest: 'check-loop-risk-health' }));
}

function unique(values) {
  return Array.from(new Set(values));
}

function printUsage() {
  console.log(`Usage:
  node scripts/check-loop-risk-health.mjs --config <path>
  node scripts/check-loop-risk-health.mjs --url <url> [options]

Canonical:
  --config <path>              Strict canonical loop-risk JSON config.

Direct diagnostic:
  --url <url>                 Explicit router loop-risk health URL.
  --runtime-id <id>           Touched runtime id. May be repeated or comma-separated.
  --runtime-ids <ids>         Comma-separated touched runtime ids.

Polling:
  --timeout-ms <ms>           Poll timeout. Default: 5000.
  --interval-ms <ms>          Poll interval. Default: 250.
  --self-test                 Run evaluator self-checks without file or network access.`);
}
