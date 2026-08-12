import { mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

import {
  PHASE0_COMMAND_SCHEMA,
  parseTestSummary,
  sha256,
} from './bytecode-vm-phase-0-contract.mjs';

export async function writePhase0CommandReceipt(outputDir, spec, outcome, {
  stdout = '',
  stderr = '',
  startedAt = null,
  finishedAt = null,
  interruptedBy = null,
} = {}) {
  const commandDir = join(outputDir, 'commands');
  await mkdir(commandDir, { recursive: true, mode: 0o700 });
  const paths = commandPaths(spec.id);
  await writeExclusive(join(outputDir, paths.stdout), stdout);
  await writeExclusive(join(outputDir, paths.stderr), stderr);
  const normalized = normalizeOutcome(outcome, interruptedBy);
  const receipt = {
    schemaVersion: PHASE0_COMMAND_SCHEMA,
    identity: commandIdentity(spec),
    startedAt,
    finishedAt,
    outcome: { ...normalized, status: commandStatus(normalized) },
    streams: {
      stdout: streamIdentity(paths.stdout, stdout),
      stderr: streamIdentity(paths.stderr, stderr),
    },
  };
  await writeJsonExclusive(join(outputDir, paths.receipt), receipt);
  return receipt;
}

export async function loadAndValidateCommandReceipts(outputDir, specs) {
  const failures = [];
  const records = new Map();
  const expectedReceiptNames = new Set(specs.map(({ id }) => `${id}.receipt.json`));
  const commandDir = join(outputDir, 'commands');
  const names = await readdir(commandDir).catch((error) => {
    if (error?.code === 'ENOENT') return [];
    throw error;
  });
  for (const name of names.filter((entry) => entry.endsWith('.receipt.json'))) {
    if (!expectedReceiptNames.has(name)) {
      failures.push(failure('command.unexpected', `unexpected command receipt ${name}`));
    }
  }
  for (const spec of specs) {
    const loaded = await loadReceipt(outputDir, spec);
    if (loaded.error !== null) {
      failures.push(failure('command.missing', `${spec.id}: ${loaded.error}`));
      continue;
    }
    const { receipt, stdout, stderr } = loaded;
    const commandFailures = validateReceipt(receipt, spec, stdout, stderr);
    failures.push(...commandFailures);
    records.set(spec.id, {
      receipt,
      stdout,
      stderr,
      valid: commandFailures.length === 0,
      testSummary: spec.testFormat === null
        ? null
        : parseTestSummary(spec.testFormat, `${stdout}\n${stderr}`),
    });
  }
  return { records, failures };
}

export function commandIdentity(spec) {
  return {
    id: spec.id,
    command: spec.command,
    args: [...spec.args],
    cwd: spec.cwd,
    evidenceEnv: { ...spec.evidenceEnv },
    testFormat: spec.testFormat,
  };
}

export function commandStatus(outcome) {
  if (outcome?.interruptedBy !== null) return 'INTERRUPTED';
  if (outcome?.code === 0 && outcome?.signal === null && outcome?.error === null) return 'PASS';
  return 'FAIL';
}

function validateReceipt(receipt, spec, stdout, stderr) {
  const failures = [];
  if (receipt?.schemaVersion !== PHASE0_COMMAND_SCHEMA) {
    failures.push(failure('command.schema', `${spec.id} has a stale receipt schema`));
  }
  if (JSON.stringify(receipt?.identity) !== JSON.stringify(commandIdentity(spec))) {
    failures.push(failure('command.identity', `${spec.id} command identity drifted`));
  }
  const paths = commandPaths(spec.id);
  for (const [name, text] of [['stdout', stdout], ['stderr', stderr]]) {
    const observed = receipt?.streams?.[name];
    if (observed?.path !== paths[name]
      || observed?.bytes !== Buffer.byteLength(text)
      || observed?.sha256 !== sha256(text)) {
      failures.push(failure('command.stream', `${spec.id} ${name} log does not match receipt`));
    }
  }
  const status = commandStatus(receipt?.outcome);
  if (receipt?.outcome?.status !== status) {
    failures.push(failure('command.outcome', `${spec.id} has an inconsistent outcome`));
  }
  if (status === 'INTERRUPTED') {
    failures.push(failure('command.interrupted', `${spec.id} was interrupted`));
  } else if (status !== 'PASS') {
    failures.push(failure('command.failed', `${spec.id} failed`));
  }
  if (status === 'PASS' && spec.testFormat !== null) {
    const summary = parseTestSummary(spec.testFormat, `${stdout}\n${stderr}`);
    if (summary?.valid !== true) {
      failures.push(failure('command.test-count', `${spec.id} test summary is not exact and complete`));
    }
  }
  return failures;
}

async function loadReceipt(outputDir, spec) {
  const paths = commandPaths(spec.id);
  try {
    const [receiptText, stdout, stderr] = await Promise.all([
      readFile(join(outputDir, paths.receipt), 'utf8'),
      readFile(join(outputDir, paths.stdout), 'utf8'),
      readFile(join(outputDir, paths.stderr), 'utf8'),
    ]);
    return { receipt: JSON.parse(receiptText), stdout, stderr, error: null };
  } catch (error) {
    return { receipt: null, stdout: null, stderr: null, error: error?.code ?? error?.message };
  }
}

function commandPaths(id) {
  if (!/^[a-z0-9-]+$/.test(id)) throw new Error(`invalid command id ${id}`);
  return {
    stdout: `commands/${id}.stdout.log`,
    stderr: `commands/${id}.stderr.log`,
    receipt: `commands/${id}.receipt.json`,
  };
}

function normalizeOutcome(outcome, interruptedBy) {
  return {
    code: Number.isInteger(outcome?.code) ? outcome.code : null,
    signal: typeof outcome?.signal === 'string' ? outcome.signal : null,
    error: outcome?.error == null
      ? null
      : outcome.error instanceof Error ? outcome.error.message : String(outcome.error),
    interruptedBy: typeof interruptedBy === 'string' ? interruptedBy : null,
  };
}

function streamIdentity(path, value) {
  return { path, bytes: Buffer.byteLength(value), sha256: sha256(value) };
}

function failure(code, message) {
  return { code, message };
}

async function writeExclusive(path, value) {
  await writeFile(path, value, { encoding: 'utf8', flag: 'wx', mode: 0o600 });
}

export async function writeJsonExclusive(path, value) {
  await writeExclusive(path, `${JSON.stringify(value, null, 2)}\n`);
}
