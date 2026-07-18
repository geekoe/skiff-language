import assert from 'node:assert/strict';
import test from 'node:test';

import {
  resolveConfiguredPackages,
  runCratePublicApiGate,
} from '../lib/crate-public-api-gate.mjs';
import { MANAGED_CRATE_NAMES } from '../lib/crate-public-api-policy.mjs';

test('all-configured gate owns one metadata/probe session and streams crates in policy order', async () => {
  const calls = [];
  const events = [];
  const packages = MANAGED_CRATE_NAMES.map(packageInfo);
  const dependencies = fakeDependencies(calls, { packages, nightlyAvailable: false });
  const outcome = await runCratePublicApiGate({
    dependencies,
    env: { PATH: '/fake/bin' },
    options: allConfiguredOptions(),
    report(event) {
      calls.push(`report:${event.kind}:${event.crateName}`);
      events.push(event);
    },
    root: '/repo',
  });

  assert.deepEqual(outcome, { exitCode: 0, violationCount: 0 });
  assert.equal(calls.filter((call) => call === 'metadata').length, 1);
  assert.equal(calls.filter((call) => call === 'probe').length, 1);
  assert.deepEqual(
    calls.filter((call) => call.startsWith('build:')).map((call) => call.slice(6)),
    MANAGED_CRATE_NAMES,
  );
  assert.deepEqual(
    events.filter((event) => event.kind === 'warning').map((event) => event.code),
    MANAGED_CRATE_NAMES.map(() => 'nightly-unavailable'),
  );
  assert.deepEqual(
    events.filter((event) => event.kind === 'crate-result').map((event) => event.crateName),
    MANAGED_CRATE_NAMES,
  );
  for (const crateName of MANAGED_CRATE_NAMES) {
    assertOrdered(calls, [
      `report:warning:${crateName}`,
      `build:${crateName}`,
      `read:${crateName}`,
      `check:${crateName}`,
      `report:crate-result:${crateName}`,
    ]);
  }
});

test('policy violations continue later crates and classify the final exit once', async () => {
  const calls = [];
  const packages = MANAGED_CRATE_NAMES.map(packageInfo);
  const dependencies = fakeDependencies(calls, {
    packages,
    violationsByCrate: new Map([[MANAGED_CRATE_NAMES[1], [{ site: 'denied' }]]]),
  });
  const events = [];
  const outcome = await runCratePublicApiGate({
    dependencies,
    options: allConfiguredOptions(),
    report: (event) => events.push(event),
    root: '/repo',
  });

  assert.deepEqual(outcome, { exitCode: 1, violationCount: 1 });
  assert.equal(
    calls.filter((call) => call.startsWith('build:')).length,
    MANAGED_CRATE_NAMES.length,
  );
  assert.equal(
    events.filter((event) => event.kind === 'crate-result').length,
    MANAGED_CRATE_NAMES.length,
  );
});

test('gate emits typed fallback warning before rustdoc read and result report', async () => {
  const calls = [];
  const events = [];
  const crateName = 'explicit-crate';
  const dependencies = fakeDependencies(calls, {
    fallbackCrate: crateName,
    packages: [packageInfo(crateName)],
  });
  await runCratePublicApiGate({
    dependencies,
    options: { allConfigured: false, crateName, extraAllowedCrates: [] },
    report(event) {
      calls.push(`report:${event.kind}:${event.code ?? event.crateName}`);
      events.push(event);
    },
    root: '/repo',
  });

  assert.deepEqual(events[0], {
    kind: 'warning',
    code: 'rustdoc-fallback-succeeded',
    crateName,
    label: 'RUSTC_BOOTSTRAP=1 cargo rustdoc',
  });
  assert.equal(events[1].kind, 'crate-result');
  assertOrdered(calls, [
    `build:${crateName}`,
    'report:warning:rustdoc-fallback-succeeded',
    `read:${crateName}`,
    `report:crate-result:${crateName}`,
  ]);
});

test('operational failure preserves prior reports and stops the serial session immediately', async () => {
  const calls = [];
  const events = [];
  const failedCrate = MANAGED_CRATE_NAMES[0];
  const dependencies = fakeDependencies(calls, {
    buildFailure: failedCrate,
    packages: MANAGED_CRATE_NAMES.map(packageInfo),
  });

  await assert.rejects(
    runCratePublicApiGate({
      dependencies,
      options: allConfiguredOptions(),
      report: (event) => events.push(event),
      root: '/repo',
    }),
    /synthetic rustdoc failure/,
  );
  assert.deepEqual(
    events.filter((event) => event.kind === 'crate-result').map((event) => event.crateName),
    [],
  );
  assert.deepEqual(
    calls.filter((call) => call.startsWith('build:')).map((call) => call.slice(6)),
    [failedCrate],
  );
  assert.equal(calls.includes(`build:${MANAGED_CRATE_NAMES[1]}`), false);
});

test('configured package resolution fails closed before probe and explicit absence is a skip', async () => {
  assert.throws(
    () => resolveConfiguredPackages(
      { packages: MANAGED_CRATE_NAMES.slice(1).map(packageInfo) },
      MANAGED_CRATE_NAMES,
    ),
    /configured public API crate\(s\) missing.*compiler-contract/,
  );

  const missingCalls = [];
  await assert.rejects(
    runCratePublicApiGate({
      dependencies: fakeDependencies(missingCalls, {
        packages: MANAGED_CRATE_NAMES.slice(1).map(packageInfo),
      }),
      options: allConfiguredOptions(),
      report() {},
      root: '/repo',
    }),
    /configured public API crate\(s\) missing/,
  );
  assert.deepEqual(missingCalls, ['metadata']);

  const skipCalls = [];
  const skipEvents = [];
  const outcome = await runCratePublicApiGate({
    dependencies: fakeDependencies(skipCalls, { packages: [] }),
    options: {
      allConfigured: false,
      crateName: 'not-present',
      extraAllowedCrates: [],
    },
    report: (event) => skipEvents.push(event),
    root: '/repo',
  });
  assert.deepEqual(outcome, { exitCode: 0, violationCount: 0 });
  assert.deepEqual(skipCalls, ['metadata']);
  assert.deepEqual(skipEvents, [{ kind: 'skip', crateName: 'not-present' }]);
});

function fakeDependencies(calls, {
  buildFailure,
  fallbackCrate,
  nightlyAvailable = true,
  packages,
  violationsByCrate = new Map(),
}) {
  return {
    async cargoMetadata() {
      calls.push('metadata');
      return { packages };
    },
    async probeCargoNightly() {
      calls.push('probe');
      return { available: nightlyAvailable };
    },
    async buildRustdocJson({ crateName }) {
      calls.push(`build:${crateName}`);
      if (crateName === buildFailure) {
        throw new Error('synthetic rustdoc failure');
      }
      return crateName === fallbackCrate
        ? { fallbackLabel: 'RUSTC_BOOTSTRAP=1 cargo rustdoc' }
        : {};
    },
    async readRustdocJson({ packageInfo: { name } }) {
      calls.push(`read:${name}`);
      return { crateName: name };
    },
    checkPublicApi(rustdoc, config) {
      calls.push(`check:${rustdoc.crateName}`);
      return {
        crateName: rustdoc.crateName,
        config,
        violations: violationsByCrate.get(rustdoc.crateName) ?? [],
      };
    },
  };
}

function allConfiguredOptions() {
  return { allConfigured: true, crateName: undefined, extraAllowedCrates: [] };
}

function packageInfo(name) {
  return { name };
}

function assertOrdered(values, expected) {
  let previous = -1;
  for (const value of expected) {
    const index = values.indexOf(value);
    assert.ok(index > previous, `${value} is out of order in ${values.join(', ')}`);
    previous = index;
  }
}
