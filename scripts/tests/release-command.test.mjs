import assert from 'node:assert/strict';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { test } from 'node:test';

import {
  encodeServiceSegment,
  locateDeploymentRecord,
  parseReleaseArgs,
  releaseCommandUsage,
  releaseCompilerInvocation,
  renderReleaseReceipt,
  runReleaseCommand,
  serviceDeploymentRefJson,
  validateBuildId,
  validateVersionSegment,
} from '../lib/release-command.mjs';

const skiffRoot = '/workspace/skiff';
const artifactRoot = '/workspace/artifacts';
const buildId = `skiff-deployment-artifact-v4:sha256:${'a'.repeat(64)}`;
const hex = 'a'.repeat(64);
const expectedPointer = `{"schemaVersion":"skiff-release-pointer-v1","profile":"dev","deployment":{"serviceId":"example.echo","contractVersion":"1.0.0","deploymentRevision":"revision-1","deploymentArtifactIdentity":"${buildId}"},"recordPath":"records/service-deployments/example~decho/1.0.0/revision-1/${hex}.json"}`;

function fakeCompiler(receipt, { code = 0, stdoutOverride } = {}) {
  const calls = [];
  const runCompiler = async (command, args, options) => {
    calls.push({ command, args, options });
    if (code !== 0) {
      return { error: null, signal: null, code, stdout: stdoutOverride ?? '', stderr: 'boom' };
    }
    return {
      error: null,
      signal: null,
      code: 0,
      stdout: stdoutOverride ?? JSON.stringify(receipt),
      stderr: '',
    };
  };
  return { runCompiler, calls };
}

async function tempArtifactStore() {
  const root = await mkdtemp(join(tmpdir(), 'skiff-release-test-'));
  return { root, cleanup: () => rm(root, { recursive: true, force: true }) };
}

async function writeDeploymentRecord(root, { serviceId, version, revision, buildIdHex }) {
  const directory = join(
    root,
    'records',
    'service-deployments',
    encodeServiceSegment(serviceId),
    version,
    revision,
  );
  await mkdir(directory, { recursive: true });
  await writeFile(join(directory, `${buildIdHex}.json`), '{}');
}

test('validateBuildId accepts the deployment artifact identity shape only', () => {
  assert.equal(validateBuildId(buildId), hex);
  assert.throws(() => validateBuildId('skiff-package-build-v10:sha256:' + hex), /--build-id must be a/);
  assert.throws(() => validateBuildId(`${buildId}${'b'}`), /64 lowercase hex/);
  assert.throws(() => validateBuildId(`skiff-deployment-artifact-v4:sha256:${'A'.repeat(64)}`), /64 lowercase hex/);
  assert.throws(() => validateBuildId(undefined), /--build-id must be a/);
});

test('encodeServiceSegment mirrors the artifact-identity coordinate codec', () => {
  assert.equal(encodeServiceSegment('example.com/echo'), 'example~dcom~secho');
  assert.equal(encodeServiceSegment('a.b/c/d'), 'a~db~sc~sd');
  assert.equal(encodeServiceSegment('plain'), 'plain');
  assert.throws(() => encodeServiceSegment('a~b'), /invalid service id/);
  assert.throws(() => encodeServiceSegment('/leading'), /invalid service id/);
  assert.throws(() => encodeServiceSegment('UPPER'), /invalid service id/);
  assert.throws(() => encodeServiceSegment(''), /invalid service id/);
});

test('validateVersionSegment accepts safe segments and rejects traversal', () => {
  assert.equal(validateVersionSegment('1.0.0'), '1.0.0');
  assert.equal(validateVersionSegment('v1_2'), 'v1_2');
  assert.throws(() => validateVersionSegment('..'), /invalid version/);
  assert.throws(() => validateVersionSegment('1.0/0'), /invalid version/);
  assert.throws(() => validateVersionSegment(''), /invalid version/);
});

test('parseReleaseArgs validates required options per action', () => {
  assert.deepEqual(
    parseReleaseArgs(['set', '--artifact-root', artifactRoot, '--profile', 'dev', '--service', 'example.echo', '--version', '1.0.0', '--build-id', buildId, '--json']),
    {
      action: 'set',
      artifactRoot,
      profile: 'dev',
      serviceId: 'example.echo',
      version: '1.0.0',
      buildId,
      expected: undefined,
      json: true,
    },
  );
  assert.throws(() => parseReleaseArgs(['swap', '--artifact-root', artifactRoot]), /unknown release action swap/);
  assert.throws(() => parseReleaseArgs(['set', '--profile', 'dev']), /requires --artifact-root/);
  assert.throws(() => parseReleaseArgs(['set', '--artifact-root', artifactRoot, '--profile', 'dev']), /requires --service/);
  assert.throws(() => parseReleaseArgs(['set', '--artifact-root', artifactRoot, '--profile', 'dev', '--service', 'x']), /requires --version/);
  assert.throws(() => parseReleaseArgs(['set', '--artifact-root', artifactRoot, '--profile', 'dev', '--service', 'x', '--version', '1']), /requires --build-id/);
  assert.throws(() => parseReleaseArgs(['unset', '--artifact-root', artifactRoot, '--profile', 'dev']), /requires --service/);
  assert.throws(() => parseReleaseArgs(['get', '--artifact-root', artifactRoot, '--profile', 'dev', '--service', 'x']), /requires --version/);
  assert.throws(() => parseReleaseArgs(['set', '--artifact-root', artifactRoot, '--artifact-root', artifactRoot, '--profile', 'dev']), /was provided more than once/);
  assert.throws(() => parseReleaseArgs(['set', '--artifact-root', artifactRoot, '--profile', 'dev', '--bogus']), /unknown option --bogus/);
  assert.throws(() => parseReleaseArgs(['set', 'positional']), /does not accept a positional argument/);
  assert.equal(parseReleaseArgs(['get', '--artifact-root', artifactRoot, '--profile', 'dev', '--service', 'x', '--version', '1']).json, false);
});

test('serviceDeploymentRefJson renders the exact camelCase reference', () => {
  assert.equal(
    serviceDeploymentRefJson({ serviceId: 'example.echo', version: '1.0.0', revision: 'revision-1', buildId }),
    `{"serviceId":"example.echo","contractVersion":"1.0.0","deploymentRevision":"revision-1","deploymentArtifactIdentity":"${buildId}"}`,
  );
  assert.throws(() => serviceDeploymentRefJson({ serviceId: 'x', version: '1', revision: 'r', buildId: 'bad' }));
});

test('locateDeploymentRecord requires exactly one matching revision', async () => {
  const { root, cleanup } = await tempArtifactStore();
  try {
    await writeDeploymentRecord(root, { serviceId: 'example.echo', version: '1.0.0', revision: 'revision-1', buildIdHex: hex });
    const match = await locateDeploymentRecord({ artifactRoot: root, serviceId: 'example.echo', version: '1.0.0', buildId });
    assert.equal(match.revision, 'revision-1');
    assert.equal(match.recordPath.endsWith(`/records/service-deployments/example~decho/1.0.0/revision-1/${hex}.json`), true);

    await assert.rejects(
      locateDeploymentRecord({ artifactRoot: root, serviceId: 'example.echo', version: '1.0.0', buildId: `skiff-deployment-artifact-v4:sha256:${'b'.repeat(64)}` }),
      /no deployment record for buildId/,
    );
    await assert.rejects(
      locateDeploymentRecord({ artifactRoot: root, serviceId: 'example.echo', version: '2.0.0', buildId }),
      /no deployment records for example.echo@2.0.0/,
    );

    await writeDeploymentRecord(root, { serviceId: 'example.echo', version: '1.0.0', revision: 'revision-2', buildIdHex: hex });
    await assert.rejects(
      locateDeploymentRecord({ artifactRoot: root, serviceId: 'example.echo', version: '1.0.0', buildId }),
      /ambiguous deployment record for buildId/,
    );
  } finally {
    await cleanup();
  }
});

test('releaseCompilerInvocation builds the exact compiler command', () => {
  const set = releaseCompilerInvocation({
    skiffRoot,
    action: 'set',
    artifactRoot,
    profile: 'dev',
    serviceId: 'example.echo',
    version: '1.0.0',
    deploymentRefJson: `{"serviceId":"example.echo"}`,
    expected: expectedPointer,
    json: true,
  });
  assert.equal(set.command, 'cargo');
  assert.equal(set.cwd, skiffRoot);
  assert.deepEqual(set.args.slice(0, 9), ['run', '--quiet', '--manifest-path', join(skiffRoot, 'compiler', 'Cargo.toml'), '--bin', 'skiff-compiler', '--', 'release', 'set']);
  assert.ok(set.args.includes('--deployment'));
  assert.ok(set.args.includes('--expected'));
  assert.equal(set.args.includes('--json'), true);

  const unset = releaseCompilerInvocation({
    skiffRoot,
    action: 'unset',
    artifactRoot,
    profile: 'dev',
    serviceId: 'example.echo',
    version: '1.0.0',
    json: false,
  });
  assert.ok(unset.args.includes('--service'));
  assert.ok(unset.args.includes('--version'));
  assert.equal(unset.args.includes('--deployment'), false);
  assert.equal(unset.args.includes('--expected'), false);
  assert.throws(() => releaseCompilerInvocation({
    skiffRoot: 'relative',
    action: 'get',
    artifactRoot,
    profile: 'dev',
    serviceId: 'x',
    version: '1',
  }), /absolute/);
});

test('runReleaseCommand resolves the record, delegates to the compiler, and renders receipts', async () => {
  const { root, cleanup } = await tempArtifactStore();
  try {
    await writeDeploymentRecord(root, { serviceId: 'example.echo', version: '1.0.0', revision: 'revision-1', buildIdHex: hex });
    const receipt = {
      action: 'set',
      pointer: {
        schemaVersion: 'skiff-release-pointer-v1',
        profile: 'dev',
        deployment: { serviceId: 'example.echo', contractVersion: '1.0.0', deploymentRevision: 'revision-1', deploymentArtifactIdentity: buildId },
        recordPath: `records/service-deployments/example~decho/1.0.0/revision-1/${hex}.json`,
      },
      pointerPath: `pointers/releases/dev/example~decho/1.0.0.json`,
    };
    const { runCompiler, calls } = fakeCompiler(receipt);
    const output = [];
    const result = await runReleaseCommand([
      'set',
      '--artifact-root',
      root,
      '--profile',
      'dev',
      '--service',
      'example.echo',
      '--version',
      '1.0.0',
      '--build-id',
      buildId,
      '--json',
    ], { skiffRoot, stdout: (line) => output.push(line), runCompiler });
    assert.deepEqual(result, receipt);
    const invocation = calls[0].args;
    const deploymentIndex = invocation.indexOf('--deployment');
    assert.deepEqual(
      JSON.parse(invocation[deploymentIndex + 1]),
      {
        serviceId: 'example.echo',
        contractVersion: '1.0.0',
        deploymentRevision: 'revision-1',
        deploymentArtifactIdentity: buildId,
      },
    );
    assert.equal(invocation.includes('--json'), true);
    assert.equal(output.length, 1);
    assert.equal(JSON.parse(output[0]).pointerPath, receipt.pointerPath);

    const { runCompiler: getCompiler, calls: getCalls } = fakeCompiler({
      action: 'get',
      profile: 'dev',
      serviceId: 'example.echo',
      version: '1.0.0',
      pointer: null,
      pointerPath: 'pointers/releases/dev/example~decho/1.0.0.json',
    });
    const getOutput = [];
    await runReleaseCommand([
      'get',
      '--artifact-root',
      root,
      '--profile',
      'dev',
      '--service',
      'example.echo',
      '--version',
      '1.0.0',
    ], { skiffRoot, stdout: (line) => getOutput.push(line), runCompiler: getCompiler });
    assert.equal(getCalls[0].args.includes('--deployment'), false);
    assert.equal(getOutput[0], 'release pointer for dev example.echo 1.0.0\n  -> (none)\npointer: pointers/releases/dev/example~decho/1.0.0.json');
  } finally {
    await cleanup();
  }
});

test('runReleaseCommand surfaces compiler failures and invalid JSON', async () => {
  const { root, cleanup } = await tempArtifactStore();
  try {
    await writeDeploymentRecord(root, { serviceId: 'example.echo', version: '1.0.0', revision: 'revision-1', buildIdHex: hex });
    const failing = fakeCompiler(null, { code: 1 });
    await assert.rejects(
      runReleaseCommand(['set', '--artifact-root', root, '--profile', 'dev', '--service', 'example.echo', '--version', '1.0.0', '--build-id', buildId], { skiffRoot, runCompiler: failing.runCompiler }),
      /release set failed: boom/,
    );
    const invalid = fakeCompiler(null, { stdoutOverride: 'not-json' });
    await assert.rejects(
      runReleaseCommand(['set', '--artifact-root', root, '--profile', 'dev', '--service', 'example.echo', '--version', '1.0.0', '--build-id', buildId], { skiffRoot, runCompiler: invalid.runCompiler }),
      /returned invalid JSON/,
    );
  } finally {
    await cleanup();
  }
});

test('releaseCommandUsage covers the three actions and help short-circuits', async () => {
  assert.match(releaseCommandUsage, /skiff release set/);
  assert.match(releaseCommandUsage, /skiff release unset/);
  assert.match(releaseCommandUsage, /skiff release get/);
  const output = [];
  const result = await runReleaseCommand(['-h'], { skiffRoot, stdout: (line) => output.push(line) });
  assert.equal(result, null);
  assert.equal(output.join('\n'), releaseCommandUsage);
});

test('renderReleaseReceipt renders set, unset, and get', () => {
  const pointer = {
    schemaVersion: 'skiff-release-pointer-v1',
    profile: 'dev',
    deployment: { serviceId: 'example.echo', contractVersion: '1.0.0', deploymentRevision: 'revision-1', deploymentArtifactIdentity: buildId },
    recordPath: `records/service-deployments/example~decho/1.0.0/revision-1/${hex}.json`,
  };
  assert.equal(
    renderReleaseReceipt({ action: 'set', pointer, pointerPath: 'pointers/releases/dev/example~decho/1.0.0.json' }),
    `release pointer set for dev example.echo 1.0.0\n  -> ${buildId}\npointer: pointers/releases/dev/example~decho/1.0.0.json`,
  );
  assert.equal(
    renderReleaseReceipt({ action: 'unset', profile: 'dev', serviceId: 'example.echo', version: '1.0.0', removedPointer: null, pointerPath: 'pointers/releases/dev/example~decho/1.0.0.json' }),
    'release pointer unset for dev example.echo 1.0.0\n  removed: (none)\npointer: pointers/releases/dev/example~decho/1.0.0.json',
  );
  assert.equal(
    renderReleaseReceipt({ action: 'get', profile: 'dev', serviceId: 'example.echo', version: '1.0.0', pointer: null, pointerPath: 'pointers/releases/dev/example~decho/1.0.0.json' }),
    'release pointer for dev example.echo 1.0.0\n  -> (none)\npointer: pointers/releases/dev/example~decho/1.0.0.json',
  );
});
