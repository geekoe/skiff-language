import {
  phase1CandidateSpecs,
  parsePhase1TestSummary,
  phase1WorkloadSpecs,
} from './bytecode-vm-phase-1-contract.mjs';
import {
  assertGitObject,
  commandEnvironmentIdentity,
  phase2ScenarioSpecs,
  phase2WorkloadSpecs,
  sha256,
  snapshotCommandEnvironment,
  validSha256,
} from './bytecode-vm-phase-2-contract.mjs';
import {
  phase3ScenarioSpecs,
  phase3WorkloadSpecs,
} from './bytecode-vm-phase-3-contract.mjs';
import {
  phase4ScenarioSpecs,
  phase4WorkloadSpecs,
} from './bytecode-vm-phase-4-contract.mjs';
import {
  phase5ScenarioSpecs,
  phase5WorkloadSpecs,
} from './bytecode-vm-phase-5-contract.mjs';

export {
  assertGitObject,
  commandEnvironmentIdentity,
  parsePhase1TestSummary as parsePhase6TestSummary,
  sha256,
  snapshotCommandEnvironment,
  validSha256,
};

export const PHASE6_COMMAND_SCHEMA = 'skiff-bytecode-vm-phase-6-command-r1-v1';
export const PHASE6_MANIFEST_SCHEMA = 'skiff-bytecode-vm-phase-6-gate-r1-v1';

export const PHASE6_REQUIRED_LANES = Object.freeze([
  'G6',
  'F6',
  'K6',
  'X6',
  'S6',
  'I6L',
  'I6R',
  'C6',
  'R6',
  'D6',
  'T6F',
  'A6',
  'T6A',
  'SENTINEL',
  'NEG',
  'RACE',
  'MEMORY',
  'ROOT',
  'CLEANUP',
  'BOUNDED-WORK',
  'QUALITY',
  'phase-5-regression',
]);

const HOST_MATRIX_TESTS = 6;
const ROUTER_MATRIX_TESTS = 6;
const CONTAINMENT_TESTS = 2;
const KERNEL_TESTS = 6;

export function phase6ScenarioSpecs(root) {
  return Object.freeze([
    phase6Spec(root, 'phase-6-gate-self-tests', 'node', [
      '--test',
      '--test-reporter=tap',
      'scripts/tests/bytecode-vm-phase-6-gate-*.test.mjs',
    ], 'node-tap', ['G6'], 22),
    hostSuite(root, 'p6-service-matrix', 'service_', HOST_MATRIX_TESTS,
      ['S6', 'F6', 'K6', 'X6']),
    hostSuite(root, 'p6-interface-local-matrix', 'interface_local_', HOST_MATRIX_TESTS,
      ['I6L', 'F6', 'K6']),
    hostSuite(root, 'p6-interface-remote-matrix', 'interface_remote_', HOST_MATRIX_TESTS,
      ['I6R', 'S6', 'F6', 'X6']),
    hostSuite(root, 'p6-callback-matrix', 'callback_', HOST_MATRIX_TESTS,
      ['C6', 'F6', 'K6', 'X6']),
    hostSuite(root, 'p6-recoverable-matrix', 'recoverable_', HOST_MATRIX_TESTS,
      ['R6', 'F6', 'K6']),
    hostSuite(root, 'p6-db-matrix', 'db_', HOST_MATRIX_TESTS,
      ['D6', 'F6', 'K6']),
    hostSuite(root, 'p6-task-host-matrix', 'task_', HOST_MATRIX_TESTS,
      ['T6F', 'R6', 'D6', 'X6']),
    routerSuite(root, 'p6-task-router-matrix', 'task_', ROUTER_MATRIX_TESTS,
      ['T6F', 'T6A']),
    hostSuite(root, 'p6-actor-host-matrix', 'actor_', HOST_MATRIX_TESTS,
      ['A6', 'R6', 'D6', 'K6', 'X6']),
    routerSuite(root, 'p6-actor-router-matrix', 'actor_', ROUTER_MATRIX_TESTS,
      ['A6', 'T6A']),
    hostSuite(root, 'p6-containment-matrix', 'containment_', CONTAINMENT_TESTS,
      ['NEG', 'F6', 'X6']),
    hostSuite(root, 'p6-kernel-focused', 'phase_6_', KERNEL_TESTS,
      ['K6', 'SENTINEL', 'RACE', 'MEMORY', 'ROOT', 'CLEANUP', 'BOUNDED-WORK']),
    phase6Spec(root, 'p6-no-verifier-structural', 'node', [
      '--input-type=module',
      '-e',
      "import('./scripts/lib/bytecode-vm-phase-6-contract.mjs').then(m => m.assertPhase6NoVerifierStructural(process.cwd())).catch(error => { console.error(error); process.exit(1); })",
    ], null, ['NEG', 'F6', 'G6']),
    phase6Spec(root, 'phase-6-fmt-check', 'cargo', [
      'fmt', '--all', '--', '--check',
    ], null, ['QUALITY']),
    phase6Spec(root, 'phase-6-clippy-check', 'cargo', [
      'clippy', '--workspace', '--all-targets', '--all-features',
    ], null, ['QUALITY']),
  ]);
}

export function phase6WorkloadSpecs(root) {
  const inherited = phase5WorkloadSpecs(root).map((entry) => {
    const id = `phase-5-regression-${entry.id}`;
    const originChain = [
      ...phase5OriginChain(root, entry),
      { phase: 6, id },
    ];
    const sourcePhase = originChain[0].phase;
    const sourceId = originChain[0].id;
    return spec({
      id,
      cwd: entry.cwd,
      command: entry.command,
      args: normalizeInheritedArgs(entry.command, entry.args),
      testFormat: entry.testFormat,
      lanes: [...entry.lanes, 'phase-5-regression'],
      expectedTests: entry.expectedTests,
      sourcePhase,
      sourceId,
      parentPhase: 5,
      parentId: entry.id,
      originChain,
    });
  });
  return Object.freeze([
    ...phase6ScenarioSpecs(root),
    ...inherited,
  ]);
}

export function phase6CandidateSpecs(root) {
  return phase1CandidateSpecs(root);
}

export function phase6WorkloadProvenance(root) {
  return phase6WorkloadSpecs(root).map((entry) => Object.freeze({
    id: entry.id,
    sourcePhase: entry.sourcePhase,
    sourceId: entry.sourceId,
    parentPhase: entry.parentPhase ?? null,
    parentId: entry.parentId ?? null,
    originChain: Object.freeze(entry.originChain.map(Object.freeze)),
  }));
}

export function phase6BoundedWorkLedger(root) {
  const bySource = new Map(
    phase6WorkloadSpecs(root).map((entry) => [
      `${entry.sourcePhase}:${entry.sourceId}`,
      entry.id,
    ]),
  );
  const required = (phase, id) => {
    const finalId = bySource.get(`${phase}:${id}`);
    if (!finalId) {
      throw new Error(`bounded-work source ${phase}:${id} is missing from Phase 6 workload`);
    }
    return finalId;
  };
  return Object.freeze({
    'p1-dispatch-fuel': Object.freeze([
      required(1, 'l4-raw-fuel-exact-boundary'),
      required(1, 'k2-deep-local-call-frame-fuel'),
    ]),
    'p2-p3-cleanup-unwind': Object.freeze([
      required(2, 'k2-lifecycle-executor'),
      required(3, 'phase-3-negative-uncaught-throw'),
      required(3, 'phase-3-negative-host-pending-throw'),
    ]),
    'p4-wake-claim': Object.freeze([
      required(4, 'phase-4-negative-cancel-before-complete'),
      required(4, 'phase-4-negative-duplicate-wake-drop'),
    ]),
    'p5-stream-pump-buffer': Object.freeze([
      required(5, 'phase-5-deterministic-tcp-upstream'),
      required(5, 'phase-5-lifecycle-race-matrix'),
    ]),
    'p6-materialization-root-walk': Object.freeze([
      'p6-service-matrix',
      'p6-db-matrix',
      'p6-kernel-focused',
    ]),
  });
}

export function assertPhase6LaneCoverage(specs) {
  const observed = new Set(specs.flatMap(({ lanes }) => lanes));
  const missing = PHASE6_REQUIRED_LANES.filter((lane) => !observed.has(lane));
  if (missing.length > 0) {
    throw new Error(`Phase 6 r1 Gate workload matrix is missing lane(s): ${missing.join(', ')}`);
  }
}

export function assertPhase6ProvenanceCoverage(specs, provenance) {
  const byId = new Map(specs.map((entry) => [entry.id, entry]));
  const seen = new Map();
  if (!Array.isArray(provenance)) {
    throw new Error('Phase 6 provenance must be an array');
  }
  for (const row of provenance) {
    if (!row || typeof row !== 'object' || typeof row.id !== 'string') {
      throw new Error('Phase 6 provenance rows require an id');
    }
    if (seen.has(row.id)) throw new Error(`duplicate Phase 6 provenance id ${row.id}`);
    seen.set(row.id, row);
    const spec = byId.get(row.id);
    if (!spec) throw new Error(`unknown Phase 6 provenance id ${row.id}`);
    if (spec.sourcePhase !== row.sourcePhase || spec.sourceId !== row.sourceId
      || spec.parentPhase !== row.parentPhase || spec.parentId !== row.parentId
      || JSON.stringify(spec.originChain) !== JSON.stringify(row.originChain)) {
      throw new Error(`provenance row does not match spec ${row.id}`);
    }
    if (!Array.isArray(row.originChain) || row.originChain.length === 0) {
      throw new Error(`provenance row ${row.id} has no origin chain`);
    }
    const phases = row.originChain.map(({ phase }) => phase);
    if (phases.some((phase) => !Number.isInteger(phase) || phase < 1 || phase > 6)) {
      throw new Error(`provenance row ${row.id} has invalid origin phase`);
    }
    for (let index = 1; index < phases.length; index += 1) {
      if (phases[index] <= phases[index - 1]) {
        throw new Error(`provenance row ${row.id} origin phases are not strictly increasing`);
      }
    }
    if (row.originChain.at(-1)?.phase !== 6 || row.originChain.at(-1)?.id !== row.id) {
      throw new Error(`provenance row ${row.id} must end with the final Phase 6 id`);
    }
  }
  for (const spec of specs) {
    if (!seen.has(spec.id)) throw new Error(`spec ${spec.id} is missing provenance`);
  }
  for (const phase of [1, 2, 3, 4, 5, 6]) {
    if (!provenance.some(({ sourcePhase }) => sourcePhase === phase)) {
      throw new Error(`Phase 6 provenance has no sourcePhase ${phase}`);
    }
  }
}

export function assertPhase6BoundedWorkCoverage(specs, ledger) {
  if (!ledger || typeof ledger !== 'object' || Array.isArray(ledger)) {
    throw new Error('Phase 6 bounded-work ledger must be an object');
  }
  const specIds = new Set(specs.map(({ id }) => id));
  const expected = Object.keys(phase6BoundedWorkLedger('/candidate')).sort();
  const actual = Object.keys(ledger).sort();
  if (JSON.stringify(expected) !== JSON.stringify(actual)) {
    throw new Error('bounded-work ledger keys drifted from the canonical obligation set');
  }
  for (const [obligation, ids] of Object.entries(ledger)) {
    if (!Array.isArray(ids) || ids.length === 0) {
      throw new Error(`bounded-work obligation ${obligation} must map to at least one spec`);
    }
    if (new Set(ids).size !== ids.length) {
      throw new Error(`bounded-work obligation ${obligation} contains duplicate spec ids`);
    }
    for (const id of ids) {
      if (!specIds.has(id)) throw new Error(`bounded-work obligation ${obligation} has unknown id ${id}`);
    }
  }
}

export async function assertPhase6NoVerifierStructural(root) {
  const { execFile } = await import('node:child_process');
  const { promisify } = await import('node:util');
  const run = promisify(execFile);
  const patterns = [
    '(^|/)(bytecode[_-]?vm[_-]?verifier|verified[_-]?bytecode|bytecode[_-]?verifier)(/|\\.)',
    'Verified(Bytecode|Artifact|Fact|Image|Execution)',
    'link[_-]>[_-]verify|link[_-]to[_-]verify',
    'test[_-]only[_-]verifier|test[_-]only[_-]legacy[_-]path',
  ];
  const roots = [
    'artifact-model/src',
    'artifact-identity/src',
    'compiler',
    'runtime',
    'router/src',
    'scripts/lib',
    'scripts/tests',
  ];
  for (const pattern of patterns) {
    const { stdout } = await run('rg', [
      '--no-heading', '--glob', '*.rs', '--glob', '*.mjs', '-l', pattern, ...roots,
    ], { cwd: root }).catch((error) => {
      if (error?.code === 1) return { stdout: '' };
      throw error;
    });
    if (stdout.trim().length > 0) {
      throw new Error(`Phase 6 verifier/dual-path structural check matched: ${stdout.trim()}`);
    }
  }
}

function phase6Spec(cwd, id, command, args, testFormat, lanes, expectedTests = undefined) {
  const entry = {
    id,
    cwd,
    command,
    args,
    testFormat,
    lanes,
    sourcePhase: 6,
    sourceId: id,
    parentPhase: null,
    parentId: null,
    originChain: [{ phase: 6, id }],
  };
  return expectedTests === undefined ? spec(entry) : Object.freeze({
    ...spec(entry),
    expectedTests,
  });
}

function hostSuite(cwd, id, filter, expectedTests, lanes) {
  const entry = phase6Spec(cwd, id, 'cargo', [
    'test', '--no-fail-fast', '--manifest-path', 'runtime/host/Cargo.toml',
    '--test', 'bytecode_vm_phase_6', filter, '--', '--nocapture',
  ], 'rust-suite', lanes);
  return Object.freeze({ ...entry, expectedTests });
}

function routerSuite(cwd, id, filter, expectedTests, lanes) {
  const entry = phase6Spec(cwd, id, 'cargo', [
    'test', '--no-fail-fast', '--manifest-path', 'router/Cargo.toml',
    '--test', 'bytecode_vm_phase_6', filter, '--', '--nocapture',
  ], 'rust-suite', lanes);
  return Object.freeze({ ...entry, expectedTests });
}

function spec(entry) {
  return Object.freeze({
    id: entry.id,
    command: entry.command,
    args: Object.freeze([...entry.args]),
    cwd: entry.cwd,
    testFormat: entry.testFormat,
    lanes: Object.freeze([...entry.lanes]),
    ...(entry.expectedTests === undefined ? {} : { expectedTests: entry.expectedTests }),
    sourcePhase: entry.sourcePhase,
    sourceId: entry.sourceId,
    parentPhase: entry.parentPhase,
    parentId: entry.parentId,
    originChain: Object.freeze(entry.originChain.map(Object.freeze)),
  });
}

function normalizeInheritedArgs(command, args) {
  if (command === 'cargo' && args[0] === 'test' && !args.includes('--no-fail-fast')) {
    return [args[0], '--no-fail-fast', ...args.slice(1)];
  }
  return [...args];
}

function phase5OriginChain(root, phase5Entry) {
  const phase5Ids = new Set(phase5ScenarioSpecs(root).map(({ id }) => id));
  if (phase5Ids.has(phase5Entry.id)) {
    return [{ phase: 5, id: phase5Entry.id }];
  }
  const phase4ById = new Map(phase4WorkloadSpecs(root).map((entry) => [
    `phase-4-regression-${entry.id}`,
    entry.id,
  ]));
  const phase4Id = phase4ById.get(phase5Entry.id);
  if (phase4Id !== undefined) {
    return [
      ...phase4OriginChain(root, phase4Id),
      { phase: 5, id: phase5Entry.id },
    ];
  }
  throw new Error(`cannot derive Phase 5 origin for ${phase5Entry.id}`);
}

function phase4OriginChain(root, phase4Id) {
  const phase4Ids = new Set(phase4ScenarioSpecs(root).map(({ id }) => id));
  if (phase4Ids.has(phase4Id)) {
    return [{ phase: 4, id: phase4Id }];
  }
  const phase3ById = new Map(phase3WorkloadSpecs(root).map((entry) => [
    `phase-3-regression-${entry.id}`,
    entry.id,
  ]));
  const phase3Id = phase3ById.get(phase4Id);
  if (phase3Id !== undefined) {
    return [
      ...phase3OriginChain(root, phase3Id),
      { phase: 4, id: phase4Id },
    ];
  }
  throw new Error(`cannot derive Phase 4 origin for ${phase4Id}`);
}

function phase3OriginChain(root, phase3Id) {
  const phase3Ids = new Set(phase3ScenarioSpecs(root).map(({ id }) => id));
  if (phase3Ids.has(phase3Id)) {
    return [{ phase: 3, id: phase3Id }];
  }
  const phase1ById = new Map(phase1WorkloadSpecs(root).map((entry) => [
    `phase-1-regression-${entry.id}`,
    entry.id,
  ]));
  const phase1Id = phase1ById.get(phase3Id);
  if (phase1Id !== undefined) {
    return [
      { phase: 1, id: phase1Id },
      { phase: 3, id: phase3Id },
    ];
  }
  const phase2ScenarioIds = new Set(phase2ScenarioSpecs(root).map(({ id }) => id));
  const phase2ById = new Map([...phase2ScenarioIds].map((id) => [
    `phase-2-regression-${id}`,
    id,
  ]));
  const phase2Id = phase2ById.get(phase3Id);
  if (phase2Id !== undefined) {
    return [
      { phase: 2, id: phase2Id },
      { phase: 3, id: phase3Id },
    ];
  }
  throw new Error(`cannot derive Phase 3 origin for ${phase3Id}`);
}
