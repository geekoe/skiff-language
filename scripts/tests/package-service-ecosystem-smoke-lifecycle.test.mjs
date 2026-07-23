import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { dirname, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  packageServiceEcosystemSmokeExpectedMarker,
  runPackageServiceEcosystemSmoke,
} from '../lib/package-service-ecosystem-smoke-real.mjs';
import {
  readyAssemblyHealth,
  validActivationReceipt,
  validBootstrapReceipt,
  validSmokeFixtureReceipt,
} from './helpers/package-service-ecosystem-smoke-fixtures.mjs';

const checkout = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const OUTER_CLEANUP_STEPS = ['stop', 'down', 'ports', 'lease', 'workspace'];

test('ecosystem smoke bounds stalled activation, WebSocket open, and close before outer cleanup', async (t) => {
  await t.test('activation never returns', async () => {
    let activationAborts = 0;
    const outcome = await observeLifecycleFailure('f28b-never-activation', {
      activate: async ({ signal }) => {
        signal.addEventListener('abort', () => { activationAborts += 1; }, { once: true });
        return new Promise(() => {});
      },
      loadWebSocket: async () => {
        throw new Error('WebSocket must not load while activation is pending');
      },
    });

    assert.match(outcome.error.message, /ecosystem smoke I\/O deadline expired/);
    assert.equal(outcome.ioSignal.aborted, true);
    assert.equal(activationAborts, 1);
    assert.deepEqual(outcome.cleanup, OUTER_CLEANUP_STEPS);
  });

  await t.test('outer abort remains the primary activation error', async () => {
    const outerController = new AbortController();
    const primaryError = new Error('isolated owner aborted the smoke');
    const outcome = await observeLifecycleFailure('f28b-outer-abort', {
      outerController,
      activate: async () => {
        queueMicrotask(() => outerController.abort(primaryError));
        return new Promise(() => {});
      },
      loadWebSocket: async () => {
        throw new Error('WebSocket must not load after an outer abort');
      },
    });

    assert.equal(outcome.error, primaryError);
    assert.equal(outcome.ioSignal.reason, primaryError);
    assert.deepEqual(outcome.cleanup, OUTER_CLEANUP_STEPS);
  });

  await t.test('WebSocket never opens', async () => {
    const WebSocket = lifecycleWebSocket({ open: false });
    const outcome = await observeLifecycleFailure('f28b-never-open', {
      loadWebSocket: async () => WebSocket,
    });

    assert.match(outcome.error.message, /ecosystem smoke I\/O deadline expired/);
    assert.equal(outcome.ioSignal.aborted, true);
    assert.equal(WebSocket.instances.length, 1);
    assert.equal(WebSocket.instances[0].closeCalls, 0);
    assert.equal(WebSocket.instances[0].terminateCalls, 1);
    assert.deepEqual(outcome.cleanup, OUTER_CLEANUP_STEPS);
  });

  await t.test('WebSocket never closes', async () => {
    const WebSocket = lifecycleWebSocket({ close: false });
    const outcome = await observeLifecycleFailure('f28b-never-close', {
      loadWebSocket: async () => WebSocket,
    });

    assert.match(outcome.error.message, /ecosystem smoke I\/O deadline expired/);
    assert.equal(outcome.ioSignal.aborted, true);
    assert.equal(WebSocket.instances.length, 1);
    assert.equal(WebSocket.instances[0].closeCalls, 1);
    assert.equal(WebSocket.instances[0].terminateCalls, 1);
    assert.deepEqual(outcome.cleanup, OUTER_CLEANUP_STEPS);
  });

  await t.test('an earlier marker error survives stalled close cleanup', async () => {
    const terminateError = new Error('terminate cleanup failed');
    const WebSocket = lifecycleWebSocket({
      close: false,
      marker: 'wrong-primary-marker',
      terminateError,
    });
    const outcome = await observeLifecycleFailure('f28b-primary-before-close', {
      loadWebSocket: async () => WebSocket,
    });

    assert.equal(outcome.error.name, 'AssertionError');
    assert.equal(outcome.error.actual, 'wrong-primary-marker');
    assert.equal(outcome.error.expected, packageServiceEcosystemSmokeExpectedMarker);
    assert.equal(outcome.ioSignal.aborted, true);
    assert.equal(WebSocket.instances[0].closeCalls, 1);
    assert.equal(WebSocket.instances[0].terminateCalls, 1);
    assert.deepEqual(outcome.cleanup, OUTER_CLEANUP_STEPS);
  });
});

async function observeLifecycleFailure(environment, overrides = {}) {
  const cleanup = [];
  let ioSignal;
  let observedError;
  const outerController = overrides.outerController ?? new AbortController();
  const activate = overrides.activate ?? (async () => validActivationReceipt(environment));
  const runtimeOwner = async ({ runTest, validateBootstrapReceipt }) => {
    validateBootstrapReceipt(validBootstrapReceipt(environment));
    try {
      return await runTest(
        { SKIFF_TEST_ENVIRONMENT: environment },
        outerController.signal,
        {
          artifactRoot: '/isolated/artifacts',
          controlUrl: 'http://127.0.0.1:46001',
          routerHttpUrl: 'http://127.0.0.1:46000',
        },
      );
    } finally {
      cleanup.push(...OUTER_CLEANUP_STEPS);
    }
  };

  await assert.rejects(
    runPackageServiceEcosystemSmoke({
      checkout,
      replicaCount: 1,
      environment,
    }, {
      runtimeOwner,
      runCommand: async () => ({
        stdout: JSON.stringify(validSmokeFixtureReceipt(environment)),
        stderr: '',
      }),
      activate: async (input) => {
        ioSignal = input.signal;
        return activate(input);
      },
      readHealth: async () => readyAssemblyHealth(environment),
      readinessSleep: async () => {},
      loadWebSocket: overrides.loadWebSocket,
      ioTimeoutMs: 25,
    }),
    (error) => {
      observedError = error;
      return true;
    },
  );
  return { cleanup, error: observedError, ioSignal };
}

function lifecycleWebSocket({
  open = true,
  close = true,
  marker = packageServiceEcosystemSmokeExpectedMarker,
  terminateError,
} = {}) {
  return class LifecycleWebSocket extends EventEmitter {
    static CONNECTING = 0;

    static OPEN = 1;

    static CLOSED = 3;

    static instances = [];

    constructor() {
      super();
      this.readyState = LifecycleWebSocket.CONNECTING;
      this.closeCalls = 0;
      this.terminateCalls = 0;
      LifecycleWebSocket.instances.push(this);
      if (open) {
        queueMicrotask(() => {
          this.readyState = LifecycleWebSocket.OPEN;
          this.emit('open');
        });
      }
    }

    send() {
      queueMicrotask(() => this.emit('message', marker));
    }

    close() {
      this.closeCalls += 1;
      if (close) {
        this.readyState = LifecycleWebSocket.CLOSED;
        queueMicrotask(() => this.emit('close'));
      }
    }

    terminate() {
      this.terminateCalls += 1;
      if (terminateError !== undefined) throw terminateError;
      this.readyState = LifecycleWebSocket.CLOSED;
    }
  };
}
