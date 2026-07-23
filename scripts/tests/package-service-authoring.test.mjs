import assert from 'node:assert/strict';
import { cp, mkdtemp, readFile, rename, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import test from 'node:test';

import {
  compilerAuthoringInvocation,
  objectUsage,
  parseObjectArgs,
  requestAssemblyActivation,
  runCompilerAuthoring,
} from '../lib/package-service-authoring.mjs';
import {
  contractCoordinate,
  readReceiptRecord,
  writeContractRoot,
  writePackageRoot,
} from './package-service-fixtures.mjs';

const skiffRoot = resolve(import.meta.dirname, '..', '..');

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

test('public four-object CLI does not expose the internal platform trust option', () => {
  for (const kind of ['package', 'contract', 'deployment', 'assembly']) {
    assert.doesNotMatch(objectUsage(kind), /platform-source-root/);
    assert.throws(
      () => parseObjectArgs(kind, 'build', [
        '/tmp/object-root',
        '--artifact-root',
        '/tmp/artifacts',
        '--platform-source-root',
        skiffRoot,
      ]),
      /unknown option --platform-source-root/,
    );
  }
});

test('official package authority is transported only as an absolute descriptor binding', () => {
  const descriptor = '/tmp/skiff-official-package-authority.json';
  const parsed = parseObjectArgs('package', 'build', [
    '/tmp/official-package',
    '--artifact-root',
    '/tmp/artifacts',
    '--official-package-authority',
    descriptor,
  ]);
  assert.equal(parsed.officialPackageAuthority, descriptor);
  const invocation = compilerAuthoringInvocation({
    skiffRoot,
    kind: 'package',
    action: 'build',
    root: parsed.root,
    artifactRoot: parsed.artifactRoot,
    officialPackageAuthority: parsed.officialPackageAuthority,
  });
  const position = invocation.args.indexOf('--official-package-authority');
  assert.notEqual(position, -1);
  assert.equal(invocation.args[position + 1], descriptor);
  assert.equal(invocation.args.includes('--official-package-root'), false);
  assert.throws(
    () => compilerAuthoringInvocation({
      skiffRoot,
      kind: 'package',
      action: 'build',
      root: '/tmp/official-package',
      artifactRoot: '/tmp/artifacts',
      officialPackageAuthority: 'relative/authority.json',
    }),
    /descriptor must be absolute/,
  );
});

test('real compiler CLI rejects original and copied registry roots without authority', async () => {
  const temp = await mkdtemp(join(tmpdir(), 'skiff-registry-no-authority-'));
  const copiedRoot = join(temp, 'registry-copy');
  await cp(join(skiffRoot, 'registry'), copiedRoot, { recursive: true });
  for (const [name, root] of [
    ['original', join(skiffRoot, 'registry')],
    ['copy', copiedRoot],
  ]) {
    const artifactRoot = join(temp, `${name}-artifacts`);
    await assert.rejects(
      runCompilerAuthoring({
        skiffRoot,
        kind: 'package',
        action: 'build',
        root,
        artifactRoot,
      }),
      /package id skiff\.run\/registry is reserved/,
    );
    await assert.rejects(readFile(join(artifactRoot, 'store.json')), /ENOENT/);
  }
});

test('contract-first publish compiles a consumer with no provider package', async () => {
  const temp = await mkdtemp(join(tmpdir(), 'skiff-authoring-contract-first-'));
  const artifactRoot = join(temp, 'artifacts');
  const contractRoot = join(temp, 'contract');
  const consumerRoot = join(temp, 'consumer');
  await writeContractRoot(contractRoot);
  await writePackageRoot(consumerRoot, {
    packageId: 'example.com/consumer',
    contracts: [contractCoordinate()],
    api: 'run: main.run\n',
    source: 'function run() -> string { return health/health() }\n',
  });

  const contract = await runCompilerAuthoring({
    skiffRoot,
    kind: 'contract',
    action: 'publish',
    root: contractRoot,
    artifactRoot,
  });
  assert.ok(contract.serviceContractReceipt);
  assert.ok(contract.serviceContractPointerReceipt);
  assert.equal('artifactReceipt' in contract, false);
  assert.equal('pointerReceipt' in contract, false);

  const packageResult = await runCompilerAuthoring({
    skiffRoot,
    kind: 'package',
    action: 'build',
    root: consumerRoot,
    artifactRoot,
  });
  const artifact = await readReceiptRecord(artifactRoot, packageResult.packageArtifactReceipt);
  assert.equal(artifact.packageId, 'example.com/consumer');
  assert.equal(artifact.contractRequirements.length, 1);
  assert.equal(artifact.contractRequirements[0].alias, 'health');
  assert.equal(artifact.serviceRequirements.length, 1);
  assert.equal(JSON.stringify(artifact).includes('providerPackageId'), false);
  assert.equal(JSON.stringify(artifact).includes('deploymentRevision'), false);
});

test('missing and tampered published contracts fail at the compiler input boundary', async () => {
  const temp = await mkdtemp(join(tmpdir(), 'skiff-authoring-contract-negative-'));
  const contractRoot = join(temp, 'contract');
  const packageRoot = join(temp, 'package');
  const artifactRoot = join(temp, 'artifacts');
  await writeContractRoot(contractRoot);
  await writePackageRoot(packageRoot, {
    packageId: 'example.com/consumer',
    contracts: [contractCoordinate()],
    api: 'run: main.run\n',
    source: 'function run() -> string { return health/health() }\n',
  });

  await assert.rejects(
    runCompilerAuthoring({ skiffRoot, kind: 'package', action: 'build', root: packageRoot, artifactRoot }),
    /no published ServiceContract pointer/,
  );

  const published = await runCompilerAuthoring({
    skiffRoot,
    kind: 'contract',
    action: 'publish',
    root: contractRoot,
    artifactRoot,
  });
  const recordPath = join(artifactRoot, published.serviceContractReceipt.recordPath);
  const record = JSON.parse(await readFile(recordPath, 'utf8'));
  record.diagnosticText.service = 'tampered';
  await writeFile(recordPath, `${JSON.stringify(record)}\n`);
  await assert.rejects(
    runCompilerAuthoring({ skiffRoot, kind: 'package', action: 'build', root: packageRoot, artifactRoot }),
    /canonical|identity|protocol|contract dependency/i,
  );

  await rename(recordPath, `${recordPath}.hidden`);
  await assert.rejects(
    runCompilerAuthoring({ skiffRoot, kind: 'package', action: 'build', root: packageRoot, artifactRoot }),
    /read|No such file|not found/i,
  );
});

test('duplicate dependency aliases and retired options are rejected without compatibility paths', async () => {
  const temp = await mkdtemp(join(tmpdir(), 'skiff-authoring-alias-negative-'));
  const packageRoot = join(temp, 'package');
  await writePackageRoot(packageRoot, {
    packageId: 'example.com/duplicate-alias',
    contracts: [contractCoordinate('same'), {
      alias: 'same',
      serviceId: 'example.com/other',
      contractVersion: '1.0.0',
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
    /duplicate alias same/,
  );
  assert.throws(
    () => parseObjectArgs('package', 'build', [packageRoot, '--artifact-root', join(temp, 'artifacts'), '--service-artifact-root', temp]),
    /unknown option --service-artifact-root/,
  );
});

test('activation request construction rejects values outside the frozen T01 wire boundary', async () => {
  const base = {
    activationId: 'activation-1',
    expectedGeneration: 0,
    environment: 'dev',
    assembly: {
      assemblyIdentity: `skiff-runtime-assembly-v1:sha256:${'1'.repeat(64)}`,
    },
  };
  let requests = 0;
  const fetchImpl = async () => {
    requests += 1;
    return new Response('{}');
  };
  for (const override of [
    { expectedGeneration: -0 },
    { expectedGeneration: Number.MAX_SAFE_INTEGER },
    { activationId: 'not visible ascii space' },
    { environment: 'x'.repeat(201) },
    { assembly: { ...base.assembly, buildId: 'legacy' } },
  ]) {
    await assert.rejects(
      requestAssemblyActivation({ ...base, ...override, fetchImpl }),
      /activation|RuntimeAssembly/,
    );
  }
  assert.equal(requests, 0);
});

test('activation request transports its AbortSignal and preserves the abort reason', async () => {
  const controller = new AbortController();
  const primaryError = new Error('isolated smoke lifecycle expired');
  let observedSignal;
  const activation = requestAssemblyActivation({
    activationId: 'activation-signal',
    expectedGeneration: 0,
    environment: 'dev',
    assembly: {
      assemblyIdentity: `skiff-runtime-assembly-v1:sha256:${'1'.repeat(64)}`,
    },
    signal: controller.signal,
    fetchImpl: async (_url, options) => {
      observedSignal = options.signal;
      return new Promise((_resolve, reject) => {
        options.signal.addEventListener(
          'abort',
          () => reject(options.signal.reason),
          { once: true },
        );
      });
    },
  });
  controller.abort(primaryError);

  await assert.rejects(activation, (error) => error === primaryError);
  assert.equal(observedSignal, controller.signal);
});
