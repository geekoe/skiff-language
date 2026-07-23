import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';

import { captureCheckedCommand } from './command-execution.mjs';
import { runInIsolatedTestRuntime } from './isolated-test-runtime.mjs';
import { requestAssemblyActivation } from './package-service-authoring.mjs';
import {
  packageServiceEcosystemSmokeFixtureCargoArgs,
} from './package-service-ecosystem-smoke-real.mjs';
import {
  retainFixtureCargoDiagnostic,
} from './package-service-ecosystem-smoke-diagnostic.mjs';
import {
  readPackageServiceFixtureReceipt,
  validatePackageServiceActivationReceipt,
  validatePackageServiceBootstrapReceipt,
  waitForPackageServiceAssemblyReady,
} from './package-service-ecosystem-smoke-oracle.mjs';
import {
  captureI02CommittedState,
  classifyI02LoadReject,
  assertI02CommittedStateUnchanged,
  i02RuntimeAssemblyRecordPath,
  packageServiceI02SpawnSubmitBusinessResult,
  selectI02TransitivePackageRecord,
  validateI02SpawnSubmitBusinessResult,
} from './package-service-i02-combined-oracle.mjs';
import {
  createI02TransactionDeadline,
  resolveI02OwnedArtifactPath,
  withI02ArtifactRootWithdrawn,
  withTamperedI02PackageRecord,
} from './package-service-i02-combined-transaction.mjs';
import {
  requestGenerationUnary,
} from './package-service-generation-lifecycle-smoke-real.mjs';
import {
  validatePackageServiceGenerationUnaryResponse,
} from './package-service-generation-lifecycle-smoke-oracle.mjs';

const DEFAULT_TRANSACTION_DEADLINE_MS = 180_000;
const I02_FIXTURE_RELATIVE_ROOT = join(
  'test-runner',
  'fixtures',
  'package-service-i02-spawn-submit',
);
const I02_PACKAGE_ID = 'test.skiff/package-service-i02-spawn-submit';
const I02_PACKAGE_VERSION = '1.0.0';
const I02_PACKAGE_TEST_NAME =
  'I02 normal source fixture compiles canonical spawn submit';

export async function runPackageServiceI02Combined({
  checkout,
  replicaCount,
  environment,
}, dependencies = {}) {
  assert.equal(replicaCount, 1, 'I02 transaction harness owns exactly one runtime replica');
  const runtimeOwner = dependencies.runtimeOwner ?? runInIsolatedTestRuntime;
  const runCommand = dependencies.runCommand ?? captureCheckedCommand;
  const activate = dependencies.activate ?? requestAssemblyActivation;
  const requestUnary = dependencies.requestUnary ?? requestGenerationUnary;
  const validateUnary =
    dependencies.validateUnary ?? validatePackageServiceGenerationUnaryResponse;
  const waitForReady =
    dependencies.waitForReady ?? waitForPackageServiceAssemblyReady;
  const readHealth = dependencies.readHealth ?? readControlHealth;
  let bootstrapReceipt;
  const deadline = createI02TransactionDeadline({
    timeoutMs: dependencies.transactionDeadlineMs ?? DEFAULT_TRANSACTION_DEADLINE_MS,
    parentSignalTarget: dependencies.parentSignalTarget ?? process,
  });

  let ledger;
  try {
    ledger = await runtimeOwner({
      skiffRoot: checkout,
      environment,
      signalTarget: deadline.signalTarget,
      validateBootstrapReceipt: (receipt) => {
        bootstrapReceipt = validatePackageServiceBootstrapReceipt(receipt, environment);
      },
      runTest: async (isolatedEnv, signal, stack) => {
        assert.ok(
          bootstrapReceipt !== undefined,
          'I02 bootstrap receipt must be captured before authoring',
        );
        const exactCandidate = await readExactCandidate({
          checkout,
          isolatedEnv,
          signal,
          runCommand,
        });
        const fixtureRoot = resolve(checkout, I02_FIXTURE_RELATIVE_ROOT);
        let fixtureOutcome;
        try {
          fixtureOutcome = await runCommand(
            'cargo',
            packageServiceEcosystemSmokeFixtureCargoArgs({
              checkout,
              fixtureRoot,
              artifactRoot: stack.artifactRoot,
              environment,
            }),
            { cwd: checkout, env: isolatedEnv, signal },
          );
        } catch (error) {
          throw retainFixtureCargoDiagnostic(error);
        }
        const receipt = readPackageServiceFixtureReceipt(
          fixtureOutcome.stdout,
          environment,
          {
            packageId: I02_PACKAGE_ID,
            packageVersion: I02_PACKAGE_VERSION,
            packageTestName: I02_PACKAGE_TEST_NAME,
          },
        );

        const activationId = `skiff-i02-valid-${randomUUID()}`;
        const activation = await activate({
          activationUrl: `${stack.controlUrl}/__skiff/activate-assembly`,
          activationId,
          expectedGeneration: 0,
          environment,
          assembly: receipt.candidate.assembly,
          signal,
        });
        const activationResponse = validatePackageServiceActivationReceipt(activation, {
          environment,
          assemblyIdentity: receipt.candidate.assembly.assemblyIdentity,
          expectedGeneration: 0,
        });
        assert.equal(
          activation.request.activationId,
          activationId,
          'I02 valid activation receipt changed the requested activationId',
        );
        const generation = activationResponse.activeAssembly.generation;
        const readiness = await waitForReady({
          healthUrl: `${stack.controlUrl}/__router/health`,
          environment,
          generation,
          assemblyIdentity: receipt.candidate.assembly.assemblyIdentity,
          signal,
          readHealth,
        });
        const committedBefore = captureI02CommittedState(
          await readHealth(`${stack.controlUrl}/__router/health`, signal),
          {
            environment,
            generation,
            assemblyIdentity: receipt.candidate.assembly.assemblyIdentity,
            replicaId: readiness.replicaId,
          },
        );
        const unary = receipt.candidate.entrypoints[1];
        const firstResult = await requestTypedUnary({
          requestUnary,
          validateUnary,
          stack,
          unary,
          signal,
        });
        const spawnSubmit = validateI02SpawnSubmitBusinessResult(firstResult);
        const firstWithdrawal = await withI02ArtifactRootWithdrawn(
          stack,
          () => requestTypedUnary({
            requestUnary,
            validateUnary,
            stack,
            unary,
            signal,
          }),
          dependencies.rootOperations,
        );
        assert.equal(firstWithdrawal.value, firstResult);

        const assemblyRecordPath = resolveI02OwnedArtifactPath(
          stack,
          i02RuntimeAssemblyRecordPath(receipt.candidate.assembly),
        );
        const assemblyRecord = JSON.parse(await readFile(assemblyRecordPath, 'utf8'));
        const transitive = selectI02TransitivePackageRecord({
          assemblyRecord,
          candidateReceipt: receipt,
          bootstrapReceipt,
        });
        const rollbackActivationId = `skiff-i02-rollback-${randomUUID()}`;
        const tamper = await withTamperedI02PackageRecord(
          stack,
          transitive,
          async () => {
            let rejection;
            try {
              await activate({
                activationUrl: `${stack.controlUrl}/__skiff/activate-assembly`,
                activationId: rollbackActivationId,
                expectedGeneration: 1,
                environment,
                assembly: receipt.candidate.assembly,
                signal,
              });
            } catch (error) {
              rejection = error;
            }
            assert.ok(rejection !== undefined, 'I02 tampered candidate unexpectedly committed');
            return classifyI02LoadReject(rejection, {
              activationId: rollbackActivationId,
              expectedGeneration: 1,
              assemblyIdentity: receipt.candidate.assembly.assemblyIdentity,
            });
          },
          dependencies.recordOperations,
        );

        await waitForReady({
          healthUrl: `${stack.controlUrl}/__router/health`,
          environment,
          generation,
          assemblyIdentity: receipt.candidate.assembly.assemblyIdentity,
          signal,
          readHealth,
        });
        const committedAfter = captureI02CommittedState(
          await readHealth(`${stack.controlUrl}/__router/health`, signal),
          {
            environment,
            generation,
            assemblyIdentity: receipt.candidate.assembly.assemblyIdentity,
            replicaId: readiness.replicaId,
          },
        );
        assertI02CommittedStateUnchanged(committedBefore, committedAfter);
        const rollbackResult = await requestTypedUnary({
          requestUnary,
          validateUnary,
          stack,
          unary,
          signal,
        });
        assert.equal(rollbackResult, firstResult);
        const secondWithdrawal = await withI02ArtifactRootWithdrawn(
          stack,
          () => requestTypedUnary({
            requestUnary,
            validateUnary,
            stack,
            unary,
            signal,
          }),
          dependencies.rootOperations,
        );
        assert.equal(secondWithdrawal.value, firstResult);

        return {
          status: 'PASS',
          probe: 'skiff-cutover-i02-transaction',
          replicas: 1,
          exactCandidate,
          activation: {
            activationId,
            generation,
            assembly: receipt.candidate.assembly.assemblyIdentity,
            replica: readiness.replicaId,
            capability: committedBefore.capability,
          },
          positive: {
            typedUnaryResult: firstResult,
            spawnSubmit: {
              ...spawnSubmit,
              sourceFixture: I02_FIXTURE_RELATIVE_ROOT,
              workerExecutionRequired: false,
            },
            requestArtifactIo: 0,
            artifactRootWithdrawals: [
              firstWithdrawal.evidence,
              secondWithdrawal.evidence,
            ],
          },
          rollback: {
            activation: tamper.value,
            tamperedPackage: {
              ...transitive.artifact,
              recordPath: transitive.relativePath,
              recordRestored: tamper.evidence.recordRestored,
            },
            committed: committedAfter.committedTuple,
            oldTypedUnaryResult: rollbackResult,
            replica: committedAfter.replica,
            capability: committedAfter.capability,
            pendingActivation: null,
          },
          productionPath: [
            'canonicalAuthoring',
            'canonicalArtifactStore',
            'routerActivationTransaction',
            'runtimeTypedLoad',
            'runtimeUnaryBoundary',
            'normalSourceSpawnStatement',
            'runtimeCanonicalSpawnSubmit',
            'routerExactAssemblyAuthorization',
            'runtimeTypedSpawnSubmitResponse',
            'routerAbort',
          ],
          cleanup: {
            artifactRootRestored: true,
            transitiveRecordRestored: true,
            status: 'pending-isolated-runtime-shutdown',
          },
        };
      },
    });
  } finally {
    deadline.dispose();
  }
  return {
    ...ledger,
    cleanup: {
      ...ledger.cleanup,
      status: 'complete',
    },
  };
}

async function requestTypedUnary({
  requestUnary,
  validateUnary,
  stack,
  unary,
  signal,
}) {
  const response = await requestUnary({
    method: unary.method,
    url: `${stack.routerHttpUrl}${unary.path}`,
    host: unary.host,
    signal,
  });
  return validateUnary(response, packageServiceI02SpawnSubmitBusinessResult);
}

async function readExactCandidate({ checkout, isolatedEnv, signal, runCommand }) {
  const outcome = await runCommand(
    'git',
    ['rev-parse', 'HEAD', 'HEAD^{tree}', 'HEAD:Cargo.lock'],
    { cwd: checkout, env: isolatedEnv, signal },
  );
  const values = outcome.stdout.trim().split(/\s+/);
  assert.equal(values.length, 3, 'I02 exact candidate query must return commit/tree/lock');
  for (const value of values) assert.match(value, /^[0-9a-f]{40,64}$/);
  return Object.freeze({
    commit: values[0],
    tree: values[1],
    cargoLock: values[2],
  });
}

async function readControlHealth(url, signal) {
  const response = await fetch(url, { signal });
  if (!response.ok) {
    throw new Error(`I02 control health returned HTTP ${response.status}`);
  }
  return response.json();
}

export const packageServiceI02CombinedConstants = Object.freeze({
  transactionDeadlineMs: DEFAULT_TRANSACTION_DEADLINE_MS,
  fixtureRelativeRoot: I02_FIXTURE_RELATIVE_ROOT,
  packageId: I02_PACKAGE_ID,
  packageVersion: I02_PACKAGE_VERSION,
  packageTestName: I02_PACKAGE_TEST_NAME,
});
