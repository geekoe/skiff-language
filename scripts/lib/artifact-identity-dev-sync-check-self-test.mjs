import {
  collectFunctionSourceSinkFailures,
} from './artifact-identity-dev-sync-check.mjs';

export function devSyncArtifactPathSelfTestFailures() {
  const cases = [
    {
      name: 'rejects a raw assembly path hidden behind an alias',
      text: `function unsafe(root, pointer) {
  assertArtifactReferencesMatchValidated(other, validated, label);
  const artifact = pointer.serviceAssembly.assemblyPath;
  return isFile(join(root, artifact));
}`,
      expectedFailures: 1,
    },
    {
      name: 'rejects a service unit alias',
      text: `async function unsafe(root, serviceUnit) {
  const ref = serviceUnit;
  const path = ref.unitPath;
  return stat(join(root, path));
}`,
      expectedFailures: 1,
    },
    {
      name: 'rejects a package loop alias',
      text: `async function unsafe(root, packageUnits) {
  for (const unit of packageUnits) await readFile(join(root, unit.unitPath));
}`,
      expectedFailures: 1,
    },
    {
      name: 'rejects a destructured unit path and aliased filesystem sink',
      text: `async function unsafe(root, { unitPath: target }) {
  const read = readFile;
  return read(join(root, target));
}`,
      expectedFailures: 1,
    },
    {
      name: 'rejects raw access despite an unrelated assertion elsewhere',
      text: `function unrelated() {
  assertArtifactReferencesMatchValidated(actual, validated, label);
}
function unsafe(root, serviceUnit) {
  const target = serviceUnit.unitPath;
  return readFile(join(root, target));
}`,
      expectedFailures: 1,
    },
    {
      name: 'allows delegation to the dedicated helper',
      text: `async function safe(root, references, validated, label) {
  await assertValidatedArtifactClosureFiles({ root, references, validated, label });
}`,
      expectedFailures: 0,
    },
    {
      name: 'allows a pure pointer parser',
      text: `function parse(pointer) {
  return { assemblyPath: pointer.serviceAssembly.assemblyPath, unitPath: pointer.serviceUnit.unitPath };
}`,
      expectedFailures: 0,
    },
  ];
  const failures = [];
  for (const testCase of cases) {
    const actual = collectFunctionSourceSinkFailures(testCase.text, 'self-test.mjs');
    if (actual.length !== testCase.expectedFailures) {
      failures.push(
        `${testCase.name}: expected ${testCase.expectedFailures} dev-sync path failure(s), got ${actual.length}`,
      );
    }
  }
  return failures;
}
