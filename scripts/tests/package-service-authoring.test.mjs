import assert from 'node:assert/strict';
import {
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  rename,
  rm,
  symlink,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import test from 'node:test';

import {
  compilerAuthoringInvocation,
  configSnapshotAuthoringInvocation,
  objectUsage,
  parseObjectArgs,
  renderAuthoringResult,
  runConfigSnapshotAuthoring,
  runCompilerAuthoring,
} from '../lib/package-service-authoring.mjs';
import {
  contractCoordinate,
  writePackageRoot,
} from './package-service-fixtures.mjs';

const skiffRoot = resolve(import.meta.dirname, '..', '..');
const rootDeployment = {
  contractVersion: '1.0.0',
  deploymentArtifactIdentity:
    `skiff-deployment-artifact-v2:sha256:${'1'.repeat(64)}`,
  deploymentRevision: 'revision-1',
  serviceId: 'example.com/service',
};

test(
  'compiler authoring transports the exact absolute platform root independently of cwd',
  { concurrency: false },
  async () => {
    const alternateCwd = await mkdtemp(join(tmpdir(), 'skiff-authoring-cwd-'));
    const request = {
      skiffRoot,
      kind: 'package',
      action: 'build',
      root: '/tmp/example-package',
      artifactRoot: '/tmp/example-artifacts',
    };
    const originalCwd = process.cwd();
    const before = compilerAuthoringInvocation(request);
    let after;
    try {
      process.chdir(alternateCwd);
      after = compilerAuthoringInvocation(request);
    } finally {
      process.chdir(originalCwd);
    }

    assert.deepEqual(after, before);
    assert.equal(before.cwd, skiffRoot);
    const platformRootPositions = before.args
      .map((argument, index) => argument === '--platform-source-root' ? index : -1)
      .filter((index) => index !== -1);
    assert.deepEqual(platformRootPositions, [before.args.length - 3]);
    assert.equal(before.args[platformRootPositions[0] + 1], skiffRoot);
    assert.throws(
      () => compilerAuthoringInvocation({ ...request, skiffRoot: 'relative/skiff-root' }),
      /absolute skiffRoot/,
    );
  },
);

test('public package CLI does not expose the internal platform trust option', () => {
  assert.match(objectUsage('package'), /--no-bytecode/);
  assert.throws(
    () => parseObjectArgs('package', 'build', [
      '/tmp/object-root',
      '--artifact-root',
      '/tmp/artifacts',
      '--platform-source-root',
      skiffRoot,
    ]),
    /unknown option --platform-source-root/,
  );
  assert.throws(() => objectUsage('assembly'), /unsupported authoring object/);
});

test('package authoring accepts --no-bytecode as the explicit legacy opt-out', () => {
  const parsed = parseObjectArgs('package', 'build', [
    '/tmp/object-root',
    '--artifact-root',
    '/tmp/artifacts',
    '--no-bytecode',
  ]);
  assert.equal(parsed.emitBytecode, false);
  const invocation = compilerAuthoringInvocation({
    skiffRoot,
    kind: 'package',
    action: 'build',
    root: parsed.root,
    artifactRoot: parsed.artifactRoot,
    profile: parsed.profile,
    emitBytecode: parsed.emitBytecode,
  });
  assert.equal(invocation.args.includes('--no-bytecode'), true);
  const defaultInvocation = compilerAuthoringInvocation({
    skiffRoot,
    kind: 'package',
    action: 'build',
    root: parsed.root,
    artifactRoot: parsed.artifactRoot,
    profile: parsed.profile,
    emitBytecode: true,
  });
  assert.equal(defaultInvocation.args.includes('--no-bytecode'), false);
  assert.throws(
    () => parseObjectArgs('assembly', 'build', [
      '--artifact-root',
      '/tmp/artifacts',
      '--profile',
      'dev',
      '--no-bytecode',
    ]),
    /unsupported authoring object/,
  );
});

test('config snapshot production requires an explicit canonical profile', async () => {
  const base = {
    skiffRoot,
    artifactRoot: '/tmp/artifacts',

    sources: [{
      root: '/tmp/service',
      deployment: rootDeployment,
    }],
  };
  await assert.rejects(
    runConfigSnapshotAuthoring(base),
    /explicit canonical profile/,
  );
  await assert.rejects(
    runConfigSnapshotAuthoring({ ...base, profile: '..' }),
    /explicit canonical profile/,
  );
});

test('config snapshot authoring transports an explicit empty service source set', () => {
  const invocation = configSnapshotAuthoringInvocation({
    skiffRoot,
    artifactRoot: '/tmp/artifacts',
    profile: 'dev',

    sources: [],
  });
  assert.equal(invocation.args.includes('--source'), false);
  assert.deepEqual(
    invocation.args.slice(-2),
    [
      '--profile',
      'dev',
    ],
  );
});

test(
  'config snapshot CLI rejects insecure, symlink, and non-regular secret sources before cargo',
  { skip: process.platform === 'win32' },
  async () => {
    const root = await mkdtemp(join(tmpdir(), 'skiff-config-secret-mode-'));
    try {
      const secret = join(root, 'config.dev.secret.yml');
      const base = {
        skiffRoot,
        artifactRoot: '/tmp/artifacts',
        profile: 'dev',

        sources: [{
          root,
          deployment: rootDeployment,
        }],
      };
      await writeFile(secret, '"example.com/service": { apiKey: must-not-leak }\n');
      await chmod(secret, 0o644);
      await assert.rejects(
        runConfigSnapshotAuthoring(base),
        (error) => (
          /chmod 600/.test(error.message)
          && !error.message.includes('must-not-leak')
        ),
      );

      const real = join(root, 'real-secret.yml');
      await rename(secret, real);
      await symlink(real, secret);
      await assert.rejects(
        runConfigSnapshotAuthoring(base),
        /regular file, not a symlink/,
      );

      await rm(secret);
      await mkdir(secret);
      await assert.rejects(
        runConfigSnapshotAuthoring(base),
        /regular file, not a symlink/,
      );
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  },
);

test('human service API output renders the exact compiler projection', () => {
  const result = {
    serviceApiReceipt: {
      serviceId: 'example.com/account',
      serviceProtocolIdentity: 'protocol',
      projection: {
        functions: [
          {
            publicPath: 'create',
            callableId: 'create-id',
            status: 'available',
            serviceOperationId: 'create-operation',
          },
          {
            publicPath: 'inspect',
            callableId: 'inspect-id',
            status: 'available',
          },
          {
            publicPath: 'mutate',
            callableId: 'mutate-id',
            status: 'unavailable',
            reasons: ['writesCallerReachable'],
          },
        ],
      },
    },
  };
  assert.equal(
    renderAuthoringResult(result),
    'Service API for example.com/account\n'
      + 'Available: 1\n'
      + 'Package-only: 2\n'
      + '  available create\n'
      + '  package-only inspect\n'
      + '  package-only mutate\n'
      + '    - "writesCallerReachable"',
  );
  assert.equal(
    renderAuthoringResult({
      serviceApiReceipt: {
        serviceId: 'example.com/empty',
        projection: { functions: [] },
      },
    }),
    'Service API for example.com/empty\nAvailable: 0\nPackage-only: 0',
  );
  assert.equal(
    renderAuthoringResult({
      serviceApiReceipt: {
        serviceId: 'example.com/package-only',
        projection: {
          functions: [{
            publicPath: 'mutate',
            callableId: 'mutate-id',
            status: 'unavailable',
            reasons: ['writesCallerReachable'],
          }],
        },
      },
    }),
    'Service API for example.com/package-only\nAvailable: 0\nPackage-only: 1\n'
      + '  package-only mutate\n'
      + '    - "writesCallerReachable"',
  );
});

test('service package build returns one stable API receipt with operation identities', async () => {
  const temp = await mkdtemp(join(tmpdir(), 'skiff-service-api-receipt-'));
  const root = join(temp, 'service');
  await writePackageRoot(root, {
    packageId: 'example.com/ping-implementation',
    api: 'ping: main.ping\n',
    source: 'function ping() -> string { return "pong" }\n',
  });
  await writeFile(
    join(root, 'service.yml'),
    'id: example.com/ping\nserviceCalls: [ping]\n',
  );
  await writeFile(join(root, 'config.dev.yml'), [
    'timeout: 1000',
    'quota:',
    '  cpuMillis: 100',
    '  memoryBytes: 1048576',
    'principal: service:ping',
    '',
  ].join('\n'));
  const result = await runCompilerAuthoring({
    skiffRoot,
    kind: 'package',
    action: 'build',
    root,
    artifactRoot: join(temp, 'artifacts'),
  });
  assert.equal(result.serviceApiReceipt.serviceId, 'example.com/ping');
  assert.match(
    result.serviceApiReceipt.serviceProtocolIdentity,
    /^skiff-service-protocol-v6:sha256:/,
  );
  assert.deepEqual(
    result.serviceApiReceipt.projection.functions.map((entry) => ({
      path: entry.publicPath,
      status: entry.status,
      hasOperationId: typeof entry.serviceOperationId === 'string',
    })),
    [{ path: 'ping', status: 'available', hasOperationId: true }],
  );
  assert.ok(result.packageArtifactReceipt);
  assert.ok(result.serviceContractReceipt);
  assert.ok(result.serviceDeploymentReceipt);
});

test('ordinary package build emits only its PackageArtifact receipt', async () => {
  const temp = await mkdtemp(join(tmpdir(), 'skiff-ordinary-package-receipt-'));
  const root = join(temp, 'package');
  await writePackageRoot(root);
  const result = await runCompilerAuthoring({
    skiffRoot,
    kind: 'package',
    action: 'build',
    root,
    artifactRoot: join(temp, 'artifacts'),
  });
  assert.deepEqual(Object.keys(result), ['packageArtifactReceipt']);
});

test('independent contract/deployment authoring objects and mixed roots fail closed', async () => {
  const temp = await mkdtemp(join(tmpdir(), 'skiff-retired-authoring-'));
  const root = join(temp, 'service');
  await writePackageRoot(root);
  await writeFile(join(root, 'contract.yml'), '{}\n');
  await assert.rejects(
    runCompilerAuthoring({
      skiffRoot, kind: 'contract', action: 'build', root, artifactRoot: join(temp, 'artifacts'),
    }),
    /unsupported authoring object contract/,
  );
  await assert.rejects(
    runCompilerAuthoring({
      skiffRoot, kind: 'package', action: 'build', root, artifactRoot: join(temp, 'artifacts'),
    }),
    /contract\.yml is not an authoring input/,
  );
});

test('duplicate dependency aliases and retired options are rejected without compatibility paths', async () => {
  const temp = await mkdtemp(join(tmpdir(), 'skiff-authoring-alias-negative-'));
  const packageRoot = join(temp, 'package');
  await writePackageRoot(packageRoot, {
    packageId: 'example.com/duplicate-alias',
    services: [contractCoordinate('same'), {
      alias: 'same',
      id: 'example.com/other',
      version: '1.0.0',
    }],
  });
  await assert.rejects(
    runCompilerAuthoring({
      skiffRoot,
      kind: 'package',
      action: 'build',
      root: packageRoot,
      artifactRoot: join(temp, 'artifacts'),
    }),
    /alias same is assigned to more than one/,
  );
  assert.throws(
    () => parseObjectArgs('package', 'build', [packageRoot, '--artifact-root', join(temp, 'artifacts'), '--service-artifact-root', temp]),
    /unknown option --service-artifact-root/,
  );
  assert.throws(
    () => parseObjectArgs('assembly', 'build', [
      '--artifact-root',
      '/tmp/artifacts',
      '--profile',
      'dev',
      '--expected-generation',
      '0',
    ]),
    /unsupported authoring object/,
  );
  assert.throws(
    () => parseObjectArgs('assembly', 'build', [
      '--artifact-root',
      '/tmp/artifacts',
      '--profile',
      'dev',
      '--activation-url',
      'http://router.test:4101/__skiff/activate-assembly',
    ]),
    /unsupported authoring object/,
  );
});
