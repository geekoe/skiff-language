import {
  commandEnvironmentIdentity,
  parsePhase7TestSummary,
  PHASE7_COMMAND_SCHEMA,
  PHASE7_GENESIS,
  phase7EffectiveTestCount,
  phase7ExpectedTestsIdentity,
  sha256,
} from './bytecode-vm-phase-7-contract.mjs';

export function receiptBytes(receipt) {
  return `${JSON.stringify(receipt, null, 2)}\n`;
}

export function receiptDigest(receipt) {
  return sha256(receiptBytes(receipt));
}

export async function writePhase7GenesisReceipt(evidenceRoot, {
  expectedCommit,
  expectedTree,
  specCatalogDigest,
}) {
  const receipt = {
    schemaVersion: PHASE7_COMMAND_SCHEMA,
    sequence: 0,
    id: 'genesis',
    genesis: PHASE7_GENESIS,
    priorReceiptDigest: null,
    candidate: {
      commit: expectedCommit,
      tree: expectedTree,
    },
    specCatalogDigest,
  };
  await writeJsonExclusive(evidenceRoot, genesisPaths().receipt, receipt);
  return receipt;
}

export async function writePhase7CommandReceipt(evidenceRoot, spec, actualEnv, outcome, {
  sequence,
  priorReceiptDigest,
  stdout = '',
  stderr = '',
  startedAt = null,
  finishedAt = null,
  interruptedBy = null,
  blockedBy = null,
} = {}) {
  const paths = commandPaths(spec.id, sequence);
  await evidenceRoot.writeExclusive(paths.stdout, stdout);
  await evidenceRoot.writeExclusive(paths.stderr, stderr);
  const normalized = normalizeOutcome(outcome, interruptedBy, blockedBy);
  const receipt = {
    schemaVersion: PHASE7_COMMAND_SCHEMA,
    sequence,
    id: spec.id,
    priorReceiptDigest,
    identity: commandIdentity(spec, actualEnv),
    startedAt,
    finishedAt,
    outcome: { ...normalized, status: commandStatus(normalized) },
    streams: {
      stdout: streamIdentity(paths.stdout, stdout),
      stderr: streamIdentity(paths.stderr, stderr),
    },
  };
  await writeJsonExclusive(evidenceRoot, paths.receipt, receipt);
  return receipt;
}

export async function writePhase7BlockedReceipt(evidenceRoot, spec, actualEnv, {
  sequence,
  priorReceiptDigest,
  blockedBy,
  startedAt = null,
  finishedAt = null,
} = {}) {
  return writePhase7CommandReceipt(evidenceRoot, spec, actualEnv, {
    code: null,
    signal: null,
    error: null,
  }, {
    sequence,
    priorReceiptDigest,
    stdout: '',
    stderr: '',
    startedAt,
    finishedAt,
    blockedBy,
  });
}

export function phase7CommandIdentity(spec, actualEnv) {
  return commandIdentity(spec, actualEnv);
}

export async function loadAndValidatePhase7CommandReceipts(
  evidenceRoot,
  specs,
  commandEnvironments,
  {
    order,
    expectedCommit,
    expectedTree,
    specCatalogDigest,
  } = {},
) {
  const failures = [];
  const records = new Map();
  const byId = new Map(specs.map((spec) => [spec.id, spec]));
  if (!Array.isArray(order) || new Set(order).size !== order.length) {
    throw new Error('Phase 7 receipt validation requires the canonical spec order');
  }
  for (const id of order) {
    if (!byId.has(id)) throw new Error(`unknown spec id in canonical order ${id}`);
  }
  const expectedReceipts = [
    { sequence: 0, id: 'genesis' },
    ...order.map((id, index) => ({ sequence: index + 1, id })),
  ];
  const expectedNames = new Set(expectedReceipts.map(({ sequence, id }) =>
    (id === 'genesis' ? '0-genesis' : `${sequence}-${id}`)));
  const names = (await evidenceRoot.snapshotFiles())
    .map(({ path }) => path)
    .filter((path) => path.startsWith('commands/'))
    .map((path) => path.slice('commands/'.length));
  for (const name of names.filter((entry) => entry.endsWith('.receipt.json'))) {
    if (!expectedNames.has(name.slice(0, -'.receipt.json'.length))) {
      failures.push(failure('command.unexpected', `unexpected command receipt ${name}`));
    }
  }
  let previousBytesDigest = null;
  const outcomes = new Map();
  const chain = [];
  for (const { sequence, id } of expectedReceipts) {
    const spec = byId.get(id);
    const loaded = await loadReceipt(evidenceRoot, id, sequence, spec);
    if (loaded.error !== null) {
      failures.push(failure('command.missing', `${id}: ${loaded.error}`));
      continue;
    }
    const { receipt, bytes, stdout, stderr } = loaded;
    const commandFailures = validateReceipt(
      receipt,
      spec,
      spec === undefined ? undefined : commandEnvironments?.get(spec.id),
      stdout,
      stderr,
      previousBytesDigest,
      expectedCommit,
      expectedTree,
      specCatalogDigest,
      sequence,
    );
    failures.push(...commandFailures);
    const testSummary = spec?.testFormat === null
      || spec === undefined
      || receipt?.outcome?.status === 'BLOCKED'
      ? null
      : parsePhase7TestSummary(spec.testFormat, `${stdout}\n${stderr}`);
    records.set(id, {
      receipt,
      stdout,
      stderr,
      valid: commandFailures.length === 0,
      testSummary,
    });
    outcomes.set(id, receipt?.outcome?.status ?? 'MISSING');
    const digest = sha256(bytes);
    chain.push({ sequence, id, digest });
    previousBytesDigest = digest;
  }
  for (const spec of specs) {
    if (!records.has(spec.id)) {
      outcomes.set(spec.id, 'MISSING');
      continue;
    }
    const dependencyFailures = validateDependencyStatus(spec, outcomes, records);
    failures.push(...dependencyFailures);
    if (dependencyFailures.length > 0) {
      records.get(spec.id).valid = false;
    }
  }
  return { records, failures, chain };
}

function validateDependencyStatus(spec, outcomes, records) {
  const failures = [];
  const dependencies = spec.dependsOn ?? [];
  if (dependencies.length === 0) return failures;
  if (!records.has(spec.id)) return failures;
  const failedProducers = dependencies.filter((dependency) =>
    outcomes.get(dependency) !== 'PASS');
  const status = records.get(spec.id).receipt?.outcome?.status;
  if (failedProducers.length > 0) {
    if (status !== 'BLOCKED') {
      failures.push(failure('command.blocked',
        `${spec.id} consumed a failed producer without a BLOCKED receipt`));
    } else if (JSON.stringify(records.get(spec.id).receipt?.outcome?.blockedBy ?? [])
      !== JSON.stringify(failedProducers.sort())) {
      failures.push(failure('command.blocked',
        `${spec.id} BLOCKED receipt does not name its failed producers`));
    }
  } else if (status === 'BLOCKED') {
    failures.push(failure('command.blocked',
      `${spec.id} was BLOCKED without a failed producer`));
  }
  return failures;
}

function commandIdentity(spec, actualEnv) {
  const identity = {
    id: spec.id,
    command: spec.command,
    args: [...spec.args],
    cwd: spec.cwd,
    environment: commandEnvironmentIdentity(actualEnv),
    testFormat: spec.testFormat,
    lanes: [...spec.lanes],
    expectedTests: phase7ExpectedTestsIdentity(spec),
    sourcePhase: spec.sourcePhase,
    sourceId: spec.sourceId,
    parentPhase: spec.parentPhase ?? null,
    parentId: spec.parentId ?? null,
    originChain: spec.originChain,
    dependsOn: spec.dependsOn ?? [],
    producedArtifacts: spec.producedArtifacts ?? [],
    requiredArtifacts: spec.requiredArtifacts ?? [],
  };
  return identity;
}

function commandStatus(outcome) {
  if (outcome?.blockedBy !== null) return 'BLOCKED';
  if (outcome?.interruptedBy !== null) return 'INTERRUPTED';
  if (outcome?.code === 0 && outcome?.signal === null && outcome?.error === null) return 'PASS';
  return 'FAIL';
}

function validateReceipt(receipt, spec, actualEnv, stdout, stderr, previousBytesDigest,
  expectedCommit, expectedTree, specCatalogDigest, sequence) {
  const failures = [];
  if (receipt?.schemaVersion !== PHASE7_COMMAND_SCHEMA) {
    failures.push(failure('command.schema', `${receipt?.id ?? spec?.id} has a stale receipt schema`));
  }
  if (spec === undefined) {
    if (receipt?.id !== 'genesis' || receipt?.sequence !== 0) {
      failures.push(failure('command.genesis', `unexpected receipt ${receipt?.id}`));
    }
  }
  if (sequence !== receipt?.sequence) {
    failures.push(failure('command.reordered',
      `${receipt?.id ?? 'receipt'} sequence ${receipt?.sequence} does not match position ${sequence}`));
  }
  if (receipt?.id !== 'genesis' && spec !== undefined
    && receipt?.id !== spec.id) {
    failures.push(failure('command.reordered',
      `${receipt?.id} written at the canonical position of ${spec.id}`));
  }
  if (receipt?.id === 'genesis') {
    if (receipt?.genesis !== PHASE7_GENESIS
      || receipt?.candidate?.commit !== expectedCommit
      || receipt?.candidate?.tree !== expectedTree
      || receipt?.specCatalogDigest !== specCatalogDigest
      || receipt?.priorReceiptDigest !== null) {
      failures.push(failure('command.genesis',
        'genesis receipt does not bind the expected candidate and catalog'));
    }
    return failures;
  }
  if (previousBytesDigest === null
    || receipt?.priorReceiptDigest !== previousBytesDigest) {
    failures.push(failure('command.chain',
      `${receipt?.id} receipt chain is broken at position ${sequence}`));
  }
  if (actualEnv === undefined
    || JSON.stringify(receipt?.identity) !== JSON.stringify(commandIdentity(spec, actualEnv))) {
    failures.push(failure('command.identity', `${spec.id} command identity drifted`));
  }
  const paths = commandPaths(spec.id, sequence);
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
  } else if (status === 'BLOCKED') {
    if (receipt?.outcome?.code !== null || receipt?.outcome?.signal !== null
      || receipt?.outcome?.error !== null || receipt?.outcome?.interruptedBy !== null
      || stdout !== '' || stderr !== '') {
      failures.push(failure('command.blocked', `${spec.id} BLOCKED receipt carries an execution`));
    }
  } else if (status !== 'PASS') {
    failures.push(failure('command.failed', `${spec.id} failed`));
  }
  if (status === 'PASS' && spec.testFormat !== null) {
    const summary = parsePhase7TestSummary(spec.testFormat, `${stdout}\n${stderr}`);
    if (summary?.valid !== true) {
      failures.push(failure('command.test-count', `${spec.id} test summary is not exact and complete`));
    } else {
      const effective = phase7EffectiveTestCount(spec);
      if (effective !== null && summary.total !== effective) {
        failures.push(failure(
          'command.test-count',
          `${spec.id} executed ${summary.total} tests; expected exactly ${effective}`,
        ));
      }
    }
  }
  return failures;
}

async function loadReceipt(evidenceRoot, id, sequence, spec) {
  if (id === 'genesis') {
    try {
      const bytes = await evidenceRoot.readFile(genesisPaths().receipt, 'utf8');
      return { receipt: JSON.parse(bytes), bytes, stdout: '', stderr: '', error: null };
    } catch (error) {
      return { receipt: null, bytes: null, stdout: null, stderr: null, error: error?.code ?? error?.message };
    }
  }
  const paths = commandPaths(id, sequence);
  try {
    const [bytes, stdout, stderr] = await Promise.all([
      evidenceRoot.readFile(paths.receipt, 'utf8'),
      evidenceRoot.readFile(paths.stdout, 'utf8'),
      evidenceRoot.readFile(paths.stderr, 'utf8'),
    ]);
    return { receipt: JSON.parse(bytes), bytes, stdout, stderr, error: null };
  } catch (error) {
    return { receipt: null, bytes: null, stdout: null, stderr: null, error: error?.code ?? error?.message };
  }
}

function commandPaths(id, sequence) {
  if (!/^[a-z0-9-]+$/.test(id)) throw new Error(`invalid command id ${id}`);
  const base = `commands/${sequence}-${id}`;
  return {
    stdout: `${base}.stdout.log`,
    stderr: `${base}.stderr.log`,
    receipt: `${base}.receipt.json`,
  };
}

function genesisPaths() {
  return {
    receipt: 'commands/0-genesis.receipt.json',
  };
}

function normalizeOutcome(outcome, interruptedBy, blockedBy) {
  return {
    code: Number.isInteger(outcome?.code) ? outcome.code : null,
    signal: typeof outcome?.signal === 'string' ? outcome.signal : null,
    error: outcome?.error == null
      ? null
      : outcome.error instanceof Error ? outcome.error.message : String(outcome.error),
    interruptedBy: typeof interruptedBy === 'string' ? interruptedBy : null,
    blockedBy: Array.isArray(blockedBy) ? [...blockedBy].sort() : null,
  };
}

function streamIdentity(path, value) {
  return { path, bytes: Buffer.byteLength(value), sha256: sha256(value) };
}

function failure(code, message) {
  return { code, message };
}

export async function writeJsonExclusive(evidenceRoot, relativePath, value) {
  await evidenceRoot.writeExclusive(relativePath, `${JSON.stringify(value, null, 2)}\n`);
}