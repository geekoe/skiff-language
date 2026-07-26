import assert from 'node:assert/strict';
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';

import { assertValidatedArtifactClosureFiles } from '../lib/artifact-identity-dev-sync-paths.mjs';
import {
  assertArtifactReferencesMatchValidated,
  validateArtifactClosureBatch,
} from '../lib/artifact-identity-validation.mjs';

const serviceAssembly = {
  assemblyIdentity: `skiff-service-assembly-v1:sha256:${'1'.repeat(64)}`,
  assemblyPath: `assemblies/services/example~com~~service/${'1'.repeat(64)}.json`,
};
const serviceUnit = {
  schemaVersion: 'skiff-service-unit-v1',
  unitIdentity: `skiff-service-unit-v1:sha256:${'2'.repeat(64)}`,
  unitHash: '2'.repeat(64),
  unitPath: `units/services/example~com~~service/${'2'.repeat(64)}.json`,
};
const packageUnit = {
  schemaVersion: 'skiff-package-unit-v2',
  packageId: 'example.com/package',
  version: '1.0.0',
  buildIdentity: `skiff-package-build-v10:sha256:${'3'.repeat(64)}`,
  abiIdentity: `skiff-package-local-abi-v7:sha256:${'4'.repeat(64)}`,
  unitHash: '5'.repeat(64),
  unitPath: `units/packages/example~com~~package/1.0.0/${'3'.repeat(64)}.json`,
};

const references = {
  serviceAssembly,
  serviceUnit,
  packageUnits: [packageUnit],
};

const validated = {
  serviceAssembly: validatedArtifact(serviceAssembly, serviceAssembly.assemblyPath),
  serviceUnit: validatedArtifact(serviceUnit, serviceUnit.unitPath),
  packageUnits: [validatedArtifact(packageUnit, packageUnit.unitPath)],
};

test('returns complete typed contents from one CLI transaction', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-artifact-identity-adapter-'));
  const cliPath = join(root, 'identity-cli.mjs');
  const invocationPath = join(root, 'invocations.txt');
  await writeFile(cliPath, fakeIdentityCliSource(invocationPath));
  await chmod(cliPath, 0o755);
  try {
    const results = await validateArtifactClosureBatch(cliPath, [{
      key: 'service',
      artifactRoot: root,
      serviceId: 'example.com/service',
      ...references,
    }]);

    const result = results.get('service');
    assert.equal(result?.serviceAssembly.content.path, serviceAssembly.assemblyPath);
    assert.equal(result?.serviceUnit.content.path, serviceUnit.unitPath);
    assert.equal(result?.packageUnits[0]?.content.path, packageUnit.unitPath);
    assert.deepEqual(result?.packageUnits[0]?.reference, packageUnit);
    assert.equal(await readFile(invocationPath, 'utf8'), '1\n');
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('returns only CLI-validated references after an exact match', () => {
  const trusted = assertArtifactReferencesMatchValidated(
    structuredClone(references),
    validated,
    'target pointer',
  );

  assert.deepEqual(trusted, references);
});

test('rejects a reference mismatch before any filesystem access', async () => {
  const actual = structuredClone(references);
  actual.serviceAssembly.assemblyPath = '../outside.json';
  let filesystemAccesses = 0;

  await assert.rejects(
    assertValidatedArtifactClosureFiles({
      root: '/unused-artifact-root',
      references: actual,
      validated,
      label: 'target pointer',
      statPath: async () => {
        filesystemAccesses += 1;
        throw new Error('filesystem must not be reached');
      },
    }),
    /does not match validated artifact references/,
  );
  assert.equal(filesystemAccesses, 0);
});

test('checks every canonical closure file after an exact match', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-dev-sync-paths-'));
  try {
    for (const path of [
      serviceAssembly.assemblyPath,
      serviceUnit.unitPath,
      packageUnit.unitPath,
    ]) {
      await writeArtifact(root, path);
    }

    await assertValidatedArtifactClosureFiles({
      root,
      references: structuredClone(references),
      validated,
      label: 'target pointer',
    });

    await rm(join(root, packageUnit.unitPath));
    await assert.rejects(
      assertValidatedArtifactClosureFiles({
        root,
        references: structuredClone(references),
        validated,
        label: 'target pointer',
      }),
      /references missing package unit/,
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

for (const testCase of [
  {
    name: 'traversal assemblyPath',
    mutate(actual) {
      actual.serviceAssembly.assemblyPath = '../outside.json';
    },
  },
  {
    name: 'backslash assemblyPath',
    mutate(actual) {
      actual.serviceAssembly.assemblyPath = 'assemblies\\services\\outside.json';
    },
  },
  {
    name: 'different unitPath',
    mutate(actual) {
      actual.serviceUnit.unitPath = 'units/services/different.json';
    },
  },
]) {
  test(`rejects ${testCase.name} at the exact-reference boundary`, () => {
    const actual = structuredClone(references);
    testCase.mutate(actual);

    assert.throws(
      () => assertArtifactReferencesMatchValidated(actual, validated, 'target pointer'),
      /does not match validated artifact references/,
    );
  });
}

function validatedArtifact(reference, path) {
  return {
    reference,
    content: {
      path,
      value: { schemaVersion: 'fixture' },
    },
  };
}

function fakeIdentityCliSource(invocationPath) {
  return `#!/usr/bin/env node
import { appendFileSync } from 'node:fs';

appendFileSync(${JSON.stringify(invocationPath)}, '1\\n');
let input = '';
process.stdin.setEncoding('utf8');
for await (const chunk of process.stdin) input += chunk;
const request = JSON.parse(input);
process.stdout.write(JSON.stringify({
  results: request.services.map((service) => ({
    key: service.key,
    dynamicBuildId: 'skiff-service-build-v1:sha256:${'6'.repeat(64)}',
    assemblyIdentity: service.serviceAssembly.assemblyIdentity,
    serviceAssembly: {
      path: service.serviceAssembly.assemblyPath,
      value: { schemaVersion: 'skiff-assembly-v1' },
    },
    serviceUnit: {
      path: service.serviceUnit.unitPath,
      value: { schemaVersion: 'skiff-service-unit-v1' },
    },
    packageUnits: service.packageUnits.map((unit) => ({
      path: unit.unitPath,
      value: { schemaVersion: 'skiff-package-unit-v2' },
    })),
  })),
}));
`;
}

async function writeArtifact(root, artifactPath) {
  const path = join(root, artifactPath);
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, '{}\n');
}
