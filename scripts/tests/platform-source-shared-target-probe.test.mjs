import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import {
  access,
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  rm,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, join, relative } from 'node:path';
import test from 'node:test';

import {
  parseProbeArgs,
  runPlatformSourceSharedTargetProbe,
} from '../run-platform-source-shared-target-probe.mjs';

const candidate = '1'.repeat(40);
const tree = '2'.repeat(40);
const lock = '3'.repeat(40);
const prelude = 'skiff-prelude-v1:sha256:prelude-golden';
const std = 'skiff-package-build-v4:sha256:std-golden';

test('combined and full modes remain disjoint command-double orchestrations', async () => {
  const fixture = await gateFixture();
  try {
    const combinedOptions = fixture.options('combined');
    const combinedDouble = fixture.commandDouble('combined');
    const combined = await runPlatformSourceSharedTargetProbe(
      combinedOptions,
      fixture.dependencies(combinedDouble),
    );
    assert.equal(combined.status, 'PASS', combined.firstError);
    assert.equal(combined.fullProbeRuns, 0);
    assert.equal(combined.sourceSuite, null);
    assert.equal(combined.identityProbes.length, 4);
    assert.deepEqual(combined.rounds.map((round) => round.label), [
      'A-origin', 'B-origin', 'final-A-origin',
    ]);
    assert.equal(
      combinedDouble.commands.filter(isMergeOnlyFixture).length,
      1,
    );
    assert.equal(combinedDouble.commands.filter(isRunSkiffTests).length, 0);
    assert.equal(combinedDouble.commands.filter(isIdentityProbe).length, 4);
    assert.equal(combinedDouble.commands.filter(isRunnerBuild).length, 3);
    assert.equal(combinedDouble.commands.filter(isCompilerBuild).length, 3);
    assert.equal(await absent(combined.paths.aWorktree), true);
    assert.equal(await absent(combined.paths.bWorktree), true);
    assert.equal(await absent(combined.paths.taskRoot), true);
    assert.deepEqual(
      JSON.parse(await readFile(combinedOptions.ledger, 'utf8')),
      combined,
    );

    const fullOptions = fixture.options('full', {
      combinedLedger: combinedOptions.ledger,
    });
    const fullDouble = fixture.commandDouble('full', {
      combinedLedger: combinedOptions.ledger,
    });
    const full = await runPlatformSourceSharedTargetProbe(
      fullOptions,
      fixture.dependencies(fullDouble),
    );
    assert.equal(full.status, 'PASS', full.firstError);
    assert.equal(full.fullProbeRuns, 1);
    assert.deepEqual(full.sourceSuite, {
      std: { passed: 11, total: 11 },
      host: { passed: 1, total: 1 },
      finalValue: 'provider-observed-helper-mutated',
      finalValueEvidence: {
        assertionPath: join(
          fullOptions.bWorktree,
          'test-runner',
          'fixtures',
          'package-service-host',
          'consumer',
          'main.test.skiff',
        ),
        assertion: 'assert root.main.run() == "provider-observed-helper-mutated"',
      },
    });
    assert.equal(fullDouble.commands.filter(isMergeOnlyFixture).length, 0);
    assert.equal(fullDouble.commands.filter(isIdentityProbe).length, 0);
    assert.equal(fullDouble.commands.filter(isRunSkiffTests).length, 1);
    assert.equal(fullDouble.commands.filter(isRunnerBuild).length, 2);
    assert.equal(fullDouble.commands.filter(isCompilerBuild).length, 0);
    assert.deepEqual(full.rounds.map((round) => round.label), ['A-origin-full']);
    assert.equal(full.cleanup.processGroupsAbsent, true);
    assert.equal(full.cleanup.portsAbsent, true);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('candidate and combined-ledger mismatches block before build', async () => {
  const fixture = await gateFixture();
  try {
    const wrongCandidateDouble = fixture.commandDouble('combined');
    const wrongCandidate = await runPlatformSourceSharedTargetProbe(
      fixture.options('combined', { candidate: '4'.repeat(40) }),
      fixture.dependencies(wrongCandidateDouble),
    );
    assert.equal(wrongCandidate.status, 'PREFLIGHT BLOCKED');
    assert.match(wrongCandidate.primary.error, /candidate commit\/tree\/Cargo\.lock/);
    assert.equal(wrongCandidateDouble.commands.some(isBuildOrFixture), false);

    const combinedOptions = fixture.options('combined');
    const combinedDouble = fixture.commandDouble('combined');
    const combined = await runPlatformSourceSharedTargetProbe(
      combinedOptions,
      fixture.dependencies(combinedDouble),
    );
    assert.equal(combined.status, 'PASS');
    const tampered = JSON.parse(await readFile(combinedOptions.ledger, 'utf8'));
    tampered.candidate = '5'.repeat(40);
    await writeFile(combinedOptions.ledger, `${JSON.stringify(tampered)}\n`);

    const fullDouble = fixture.commandDouble('full', {
      combinedLedger: combinedOptions.ledger,
    });
    const full = await runPlatformSourceSharedTargetProbe(
      fixture.options('full', { combinedLedger: combinedOptions.ledger }),
      fixture.dependencies(fullDouble),
    );
    assert.equal(full.status, 'PREFLIGHT BLOCKED');
    assert.match(full.primary.error, /combined ledger digest mismatch/);
    assert.equal(fullDouble.commands.some(isBuildOrFixture), false);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('pre-existing cleanup targets are refused and never removed', async () => {
  const fixture = await gateFixture();
  try {
    const options = fixture.options('combined');
    await mkdir(options.aWorktree);
    const commandDouble = fixture.commandDouble('combined');
    const ledger = await runPlatformSourceSharedTargetProbe(
      options,
      fixture.dependencies(commandDouble),
    );
    assert.equal(ledger.status, 'PREFLIGHT BLOCKED');
    assert.match(ledger.primary.error, /worktree paths must be absent/);
    await access(options.aWorktree);
    assert.equal(commandDouble.commands.some(isBuildOrFixture), false);
    assert.equal(commandDouble.commands.some(({ args }) => args.includes('remove')), false);
    assert.equal(await absent(options.ledger), true);

    await rm(options.aWorktree, { recursive: true });
    await writeFile(options.ledger, 'preserve-existing-ledger\n');
    const ledgerDouble = fixture.commandDouble('combined');
    const ledgerRefusal = await runPlatformSourceSharedTargetProbe(
      options,
      fixture.dependencies(ledgerDouble),
    );
    assert.equal(ledgerRefusal.status, 'PREFLIGHT BLOCKED');
    assert.match(ledgerRefusal.primary.error, /ledger path must be absent/);
    assert.equal(await readFile(options.ledger, 'utf8'), 'preserve-existing-ledger\n');
    assert.equal(ledgerDouble.commands.some(isBuildOrFixture), false);
    assert.equal(ledgerDouble.commands.some(({ args }) => args.includes('remove')), false);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('CLI parser requires an explicit mode and mode-specific ledger option', () => {
  const common = [
    '--integration-root', '/tmp/integration',
    '--candidate', candidate,
    '--expected-tree', tree,
    '--expected-lock-blob', lock,
    '--expected-prelude-identity', prelude,
    '--expected-std-package-build-id', std,
    '--a-worktree', '/tmp/a',
    '--b-worktree', '/tmp/b',
    '--json',
  ];
  assert.throws(() => parseProbeArgs(common), /--mode/);
  assert.throws(
    () => parseProbeArgs(['--mode', 'combined', ...common]),
    /--ledger requires an absolute path/,
  );
  assert.throws(
    () => parseProbeArgs(['--mode', 'full', ...common]),
    /--combined-ledger requires an absolute path/,
  );
});

async function gateFixture() {
  const root = await realpath(
    await mkdtemp(join(tmpdir(), 'skiff-platform-probe-double-')),
  );
  const integrationRoot = join(root, 'integration');
  await mkdir(integrationRoot);
  await writeFile(join(integrationRoot, 'Cargo.lock'), 'lock\n');
  return {
    root,
    options(mode, overrides = {}) {
      return {
        mode,
        integrationRoot,
        candidate,
        expectedTree: tree,
        expectedLockBlob: lock,
        expectedPreludeIdentity: prelude,
        expectedStdPackageBuildId: std,
        aWorktree: join(root, `${mode}-a`),
        bWorktree: join(root, `${mode}-b`),
        ...(mode === 'combined'
          ? { ledger: join(integrationRoot, '.p5-i16-combined-ledger.json') }
          : { combinedLedger: join(integrationRoot, '.p5-i16-combined-ledger.json') }),
        json: true,
        ...overrides,
      };
    },
    commandDouble(mode, { combinedLedger } = {}) {
      return createCommandDouble({ integrationRoot, mode, combinedLedger });
    },
    dependencies(commandDouble) {
      return {
        signalTarget: new EventEmitter(),
        runCommand: commandDouble.run,
        availableBytes: async () => 16 * (1024 ** 3),
        allocatedBytes: async () => undefined,
        snapshotArtifacts: async (targetRoot) => fakeArtifacts(targetRoot),
        loadRegistry: async () => [{ id: 'std', root: 'std' }],
        readText: async () => [
          'test "provider observes helper mutation" {',
          '  assert root.main.run() == "provider-observed-helper-mutated"',
          '}',
          '',
        ].join('\n'),
        assertPortsClosed: async () => {},
        assertExecutables: async () => {},
      };
    },
  };
}

function createCommandDouble({ integrationRoot, mode, combinedLedger }) {
  const commands = [];
  let pid = 10_000;
  const run = async (command, args, options = {}) => {
    commands.push({ command, args: [...args], options });
    const outcome = {
      code: 0,
      signal: null,
      error: null,
      stdout: '',
      stderr: '',
      pid: pid += 1,
      processGroupAbsent: true,
      observedPorts: [],
      portsAbsent: true,
    };
    if (command === 'git' && args.includes('rev-parse')) {
      const expression = args.at(-1);
      outcome.stdout = `${expression === 'HEAD' ? candidate : expression === 'HEAD^{tree}' ? tree : lock}\n`;
    } else if (command === 'git' && args.includes('status')) {
      if (mode === 'full' && combinedLedger !== undefined) {
        outcome.stdout = `?? ${relative(integrationRoot, combinedLedger)}\n`;
      }
    } else if (command === 'git' && args.includes('worktree') && args.includes('list')) {
      outcome.stdout = `worktree ${integrationRoot}\nHEAD ${candidate}\n`;
    } else if (command === 'git' && args.includes('worktree') && args.includes('add')) {
      await mkdir(args[args.indexOf('add') + 2], { recursive: true });
    } else if (command === 'git' && args.includes('worktree') && args.includes('remove')) {
      await rm(args[args.indexOf('remove') + 2], { recursive: true, force: true });
    } else if (command === 'cargo' && args.includes('platform_source_identity_probe')) {
      outcome.stdout = [
        `PLATFORM_SOURCE_PRELUDE_IDENTITY=${prelude}`,
        `PLATFORM_SOURCE_STD_PACKAGE_BUILD_ID=${std}`,
      ].join('\n');
      outcome.stderr = freshOutput();
    } else if (command === 'cargo' && args[0] === 'build') {
      outcome.stderr = freshOutput();
    } else if (command === 'rg') {
      outcome.code = 1;
    } else if (command === 'node' && args.some((value) => value.endsWith('run-skiff-tests.mjs'))) {
      outcome.stdout = [
        'test result: ok. 11 passed; 0 failed',
        'test result: ok. 1 passed; 0 failed',
      ].join('\n');
      outcome.observedPorts = [46010, 46011, 46012];
    }
    return outcome;
  };
  return { commands, run };
}

function fakeArtifacts(targetRoot) {
  const debug = join(targetRoot, 'debug');
  const deps = join(debug, 'deps');
  return [
    structure(join(debug, 'skiff-compiler')),
    structure(join(debug, 'skiff-test-runner')),
    structure(join(debug, 'skiff-package-service-smoke-fixture')),
    structure(join(deps, 'libskiff_compiler_input-a.rlib')),
    structure(join(deps, 'libskiff_compiler_source-b.rlib')),
    structure(join(deps, 'libskiff_compiler-c.rlib')),
    {
      path: join(deps, 'package_service_contract_deployment-d'),
      sha256: 'identity',
      mtimeMs: 1,
      size: 1,
      depInfo: false,
      structureSubject: false,
      identityTest: true,
    },
    depInfo(join(deps, 'skiff_compiler_input-a.d')),
    depInfo(join(deps, 'skiff_compiler_source-b.d')),
    depInfo(join(debug, 'skiff-compiler.d')),
    depInfo(join(debug, 'skiff-test-runner.d')),
    depInfo(join(debug, 'skiff-package-service-smoke-fixture.d')),
  ];
}

function structure(path) {
  return {
    path,
    sha256: basename(path),
    mtimeMs: 1,
    size: 1,
    depInfo: false,
    structureSubject: true,
    identityTest: false,
  };
}

function depInfo(path) {
  return {
    path,
    sha256: basename(path),
    mtimeMs: 1,
    size: 1,
    depInfo: true,
    structureSubject: false,
    identityTest: false,
  };
}

function freshOutput() {
  return [
    'Fresh skiff-test-runner v0.1.0',
    'Fresh skiff-compiler v0.1.0',
    'Fresh skiff-compiler-input v0.1.0',
    'Fresh skiff-compiler-source v0.1.0',
  ].join('\n');
}

function isMergeOnlyFixture({ command, args }) {
  return command === 'node'
    && args.some((value) => value.endsWith('platform-source-transport-combined.test.mjs'));
}

function isRunSkiffTests({ command, args }) {
  return command === 'node' && args.some((value) => value.endsWith('run-skiff-tests.mjs'));
}

function isIdentityProbe({ command, args }) {
  return command === 'cargo' && args.includes('platform_source_identity_probe');
}

function isRunnerBuild({ command, args }) {
  return command === 'cargo' && args[0] === 'build'
    && args.includes('skiff-test-runner')
    && args.includes('skiff-package-service-smoke-fixture');
}

function isCompilerBuild({ command, args }) {
  return command === 'cargo' && args[0] === 'build'
    && args.includes('skiff-compiler')
    && !args.includes('skiff-test-runner');
}

function isBuildOrFixture(entry) {
  return entry.command === 'cargo' || entry.command === 'node';
}

async function absent(path) {
  try {
    await access(path);
    return false;
  } catch (error) {
    if (error.code === 'ENOENT') return true;
    throw error;
  }
}
