import { afterEach, describe, expect, it, vi } from 'vitest';
import WebSocket from 'ws';

import {
  ACTOR_RETURN_ENCODING_V1,
  decodeActorMethodFrame,
  encodeActorMethodFrame,
} from '../src/protocol/actorMethodProtocol.js';
import { decodeActorOwnerInvokeFrame } from '../src/protocol/actorOwnerProtocol.js';
import {
  decodeBinaryFrame,
  RUNTIME_FRAME_SCHEMA_VERSION,
} from '../src/protocol/envelope.js';
import { SPAWNED_ACTOR_METHOD_DEADLINE_MS } from '../src/router/actorTiming.js';
import {
  actorBootstrap,
  cleanupActorRoutingHarnesses,
  invocation,
  nextBinary,
  runtime,
  spawnContext,
  spawnHarness,
  spawnSubmit,
  waitFor,
  waitForAsync,
} from './helpers/actorRoutingHarness.js';

afterEach(cleanupActorRoutingHarnesses);

describe('actor spawn correlation lifecycle', () => {
  it('accepts a correlated owner terminal after the Router deadline settles the spawn', async () => {
    const { actorMethods, registry, left } = await spawnHarness();
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    vi.useFakeTimers();
    try {
      const result = await actorMethods.submitSpawn(
        spawnSubmit({
          runtimeId: 'runtime-a',
          callerRequestId: 'not-needed-for-direct-submit',
          actor,
        }),
        Buffer.from('[5]'),
        spawnContext(registry, 'runtime-a')
      );
      await nextBinary(left);

      const routerCancellation = nextBinary(left);
      await vi.advanceTimersByTimeAsync(SPAWNED_ACTOR_METHOD_DEADLINE_MS);
      expect(decodeActorMethodFrame(await routerCancellation).header).toMatchObject({
        type: 'actor.method.cancel',
        invocationId: result.requestId,
        reason: 'deadlineExceeded',
      });
      await expect(
        registry.actorManager().registryStore().actorInvocation(result.requestId)
      ).resolves.toMatchObject({
        state: 'cancelled',
        terminalReason: 'deadlineExceeded',
      });

      const owner = registry.runtimeConnection('runtime-a')!.ws;
      const lateTerminal = {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'actor.method.cancel' as const,
        invocationId: result.requestId,
        cancellationCorrelation: `${result.requestId}:cancel`,
        reason: 'deadlineExceeded' as const,
      };
      vi.useRealTimers();
      const closed = new Promise<boolean>((resolve) => {
        left.once('close', () => resolve(true));
      });
      left.send(encodeActorMethodFrame(lateTerminal));
      await expect(Promise.race([
        closed,
        new Promise<boolean>((resolve) => setTimeout(() => resolve(false), 50)),
      ])).resolves.toBe(false);
      expect(registry.runtimeConnection('runtime-a')?.ws).toBe(owner);
      await expect(
        actorMethods.handleFrame(owner, {
          ...lateTerminal,
          cancellationCorrelation: 'not-the-admitted-correlation',
        }, new Uint8Array())
      ).rejects.toThrow(`unknown Actor invocation ${result.requestId}`);
    } finally {
      vi.useRealTimers();
    }
  });

  it('retains owner terminal correlation when a pending or settled caller disconnects', async () => {
    const { actorMethods, registry, left, url } = await spawnHarness();
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    const ownerServer = registry.runtimeConnection('runtime-a')!.ws;
    const warmup = invocation(actor, 'caller-disconnect-warmup');
    const warmupOwnerInvoke = nextBinary(left);
    await actorMethods.handleFrame(ownerServer, warmup, Buffer.from('[0]'));
    await warmupOwnerInvoke;
    const warmupReturn = nextBinary(left);
    await actorMethods.handleFrame(ownerServer, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: warmup.invocationId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }, new Uint8Array());
    await warmupReturn;
    const caller = await runtime(url, 'runtime-b');
    const callerServer = registry.runtimeConnection('runtime-b')!.ws;

    const pendingDisconnect = invocation(actor, 'caller-disconnect-pending');
    const pendingOwnerInvoke = nextBinary(left);
    await actorMethods.handleFrame(
      callerServer,
      pendingDisconnect,
      Buffer.from('[7]')
    );
    await pendingOwnerInvoke;
    const cancellation = nextBinary(left);
    await actorMethods.handleRuntimeDisconnect(callerServer);
    expect(decodeActorMethodFrame(await cancellation).header).toMatchObject({
      type: 'actor.method.cancel',
      invocationId: pendingDisconnect.invocationId,
    });

    await expect(actorMethods.handleFrame(ownerServer, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: pendingDisconnect.invocationId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }, new Uint8Array())).resolves.toBeUndefined();
    await expect(actorMethods.handleFrame(ownerServer, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.error',
      invocationId: pendingDisconnect.invocationId,
      error: {
        name: 'actorUpgradingError',
        actorRef: pendingDisconnect.actorRef,
        retryAfterMs: 1,
      },
    }, new Uint8Array())).resolves.toBeUndefined();
    await expect(actorMethods.handleFrame(ownerServer, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.cancel',
      invocationId: pendingDisconnect.invocationId,
      cancellationCorrelation: pendingDisconnect.cancellationCorrelation,
      reason: 'cancelled',
    }, new Uint8Array())).resolves.toBeUndefined();
    const ownerClosed = new Promise<boolean>((resolve) => {
      left.once('close', () => resolve(true));
    });
    left.send(encodeActorMethodFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: pendingDisconnect.invocationId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }));
    left.send(encodeActorMethodFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.error',
      invocationId: pendingDisconnect.invocationId,
      error: {
        name: 'actorUpgradingError',
        actorRef: pendingDisconnect.actorRef,
        retryAfterMs: 1,
      },
    }));
    left.send(encodeActorMethodFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.cancel',
      invocationId: pendingDisconnect.invocationId,
      cancellationCorrelation: pendingDisconnect.cancellationCorrelation,
      reason: 'cancelled',
    }));
    await expect(Promise.race([
      ownerClosed,
      new Promise<boolean>((resolve) => setTimeout(() => resolve(false), 50)),
    ])).resolves.toBe(false);
    const ownerRoundTrip = invocation(actor, 'owner-after-late-terminals');
    const roundTripOwnerInvoke = nextBinary(left);
    left.send(encodeActorMethodFrame(ownerRoundTrip, Buffer.from('[10]')));
    expect(
      decodeActorOwnerInvokeFrame(await roundTripOwnerInvoke).header.invoke
        .invocationId
    ).toBe(ownerRoundTrip.invocationId);
    const roundTripReturn = nextBinary(left);
    left.send(encodeActorMethodFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: ownerRoundTrip.invocationId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }));
    expect(decodeActorMethodFrame(await roundTripReturn).header).toMatchObject({
      type: 'actor.method.return',
      invocationId: ownerRoundTrip.invocationId,
    });

    const settledDisconnect = invocation(actor, 'caller-disconnect-settled');
    const settledOwnerInvoke = nextBinary(left);
    await actorMethods.handleFrame(
      callerServer,
      settledDisconnect,
      Buffer.from('[8]')
    );
    await settledOwnerInvoke;
    const forwardedCancellation = nextBinary(left);
    await actorMethods.handleFrame(callerServer, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.cancel',
      invocationId: settledDisconnect.invocationId,
      cancellationCorrelation: settledDisconnect.cancellationCorrelation,
      reason: 'cancelled',
    }, new Uint8Array());
    await forwardedCancellation;
    await actorMethods.handleRuntimeDisconnect(callerServer);
    await expect(actorMethods.handleFrame(ownerServer, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: settledDisconnect.invocationId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }, new Uint8Array())).resolves.toBeUndefined();

    expect(registry.runtimeConnection('runtime-a')?.ws).toBe(ownerServer);
    expect(left.readyState).toBe(WebSocket.OPEN);
    expect(caller.readyState).toBe(WebSocket.OPEN);
  });

  it.each([
    { terminalKind: 'return' as const, terminalState: 'completed' as const },
    { terminalKind: 'error' as const, terminalState: 'failed' as const },
  ])('claims an owner $terminalKind before a delayed ledger transition can race the deadline', async ({
    terminalKind,
    terminalState,
  }) => {
    const { actorMethods, registry, left } = await spawnHarness();
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    const store = registry.actorManager().registryStore();
    const originalTransition = store.transitionActorInvocation.bind(store);
    let releaseTransition!: () => void;
    const transitionBlocked = new Promise<void>((resolve) => {
      releaseTransition = resolve;
    });
    let reportTransitionStarted!: () => void;
    const transitionStarted = new Promise<void>((resolve) => {
      reportTransitionStarted = resolve;
    });
    const invocationId = `terminal-deadline-race-${terminalKind}`;
    store.transitionActorInvocation = async (input) => {
      if (
        input.invocationId === invocationId &&
        input.nextState === terminalState
      ) {
        reportTransitionStarted();
        await transitionBlocked;
      }
      return originalTransition(input);
    };

    vi.useFakeTimers();
    try {
      const invoke = {
        ...invocation(actor, invocationId),
        deadline: {
          timeoutMs: 100,
          expiresAt: new Date(Date.now() + 100).toISOString(),
        },
      };
      const ownerServer = registry.runtimeConnection('runtime-a')!.ws;
      await actorMethods.handleFrame(ownerServer, invoke, Buffer.from('[9]'));
      await nextBinary(left);

      const forwardedTerminal = nextBinary(left);
      const terminal = terminalKind === 'return'
        ? {
            schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
            type: 'actor.method.return' as const,
            invocationId: invoke.invocationId,
            returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
          }
        : {
            schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
            type: 'actor.method.error' as const,
            invocationId: invoke.invocationId,
            error: {
              name: 'actorUpgradingError' as const,
              actorRef: invoke.actorRef,
              retryAfterMs: 1,
            },
          };
      const handling = actorMethods.handleFrame(
        ownerServer,
        terminal,
        new Uint8Array()
      );
      await transitionStarted;
      await vi.advanceTimersByTimeAsync(100);
      releaseTransition();
      await handling;

      expect(decodeActorMethodFrame(await forwardedTerminal).header).toMatchObject({
        type: `actor.method.${terminalKind}`,
        invocationId: invoke.invocationId,
      });
      await expect(store.actorInvocation(invoke.invocationId)).resolves.toMatchObject({
        state: terminalState,
      });
    } finally {
      releaseTransition();
      vi.useRealTimers();
    }
  });

  it('claims an owner failure before a delayed ledger transition can race the deadline', async () => {
    const { actorMethods, registry, left } = await spawnHarness();
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    const store = registry.actorManager().registryStore();
    const originalTransition = store.transitionActorInvocation.bind(store);
    let releaseTransition!: () => void;
    const transitionBlocked = new Promise<void>((resolve) => {
      releaseTransition = resolve;
    });
    let reportTransitionStarted!: () => void;
    const transitionStarted = new Promise<void>((resolve) => {
      reportTransitionStarted = resolve;
    });
    const invocationId = 'owner-failure-deadline-race';
    store.transitionActorInvocation = async (input) => {
      if (input.invocationId === invocationId && input.nextState === 'failed') {
        reportTransitionStarted();
        await transitionBlocked;
      }
      return originalTransition(input);
    };

    vi.useFakeTimers();
    try {
      const invoke = {
        ...invocation(actor, invocationId),
        deadline: {
          timeoutMs: 100,
          expiresAt: new Date(Date.now() + 100).toISOString(),
        },
      };
      const ownerServer = registry.runtimeConnection('runtime-a')!.ws;
      await actorMethods.handleFrame(ownerServer, invoke, Buffer.from('[11]'));
      const dispatched = decodeActorOwnerInvokeFrame(await nextBinary(left));
      const forwardedFailure = nextBinary(left);
      const handling = actorMethods.handleOwnerFailure(ownerServer, {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'actor.owner.failure',
        invocationId,
        ownerRuntimeId: dispatched.header.ownerFence.ownerRuntimeId,
        ownerLeaseId: dispatched.header.ownerFence.ownerLeaseId,
        epoch: dispatched.header.ownerFence.epoch,
        actorImplementationIdentity:
          dispatched.header.ownerFence.actorImplementationIdentity,
        reason: { code: 'InvocationFailed', message: 'delayed failure' },
      });
      await transitionStarted;
      await vi.advanceTimersByTimeAsync(100);
      releaseTransition();
      await handling;

      expect(decodeBinaryFrame(await forwardedFailure).header).toMatchObject({
        type: 'actor.owner.failure',
        invocationId,
      });
      await expect(store.actorInvocation(invocationId)).resolves.toMatchObject({
        state: 'failed',
        terminalReason: 'InvocationFailed: delayed failure',
      });
    } finally {
      releaseTransition();
      vi.useRealTimers();
    }
  });

  it('preserves exactly one winner between caller cancellation and the deadline', async () => {
    const { actorMethods, registry, left } = await spawnHarness();
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    const source = registry.runtimeConnection('runtime-a')!.ws;
    vi.useFakeTimers();
    try {
      const callerWins = {
        ...invocation(actor, 'caller-wins-deadline-race'),
        deadline: {
          timeoutMs: 100,
          expiresAt: new Date(Date.now() + 100).toISOString(),
        },
      };
      const callerWinsOwnerInvoke = nextBinary(left);
      await actorMethods.handleFrame(source, callerWins, Buffer.from('[12]'));
      await callerWinsOwnerInvoke;
      const callerCancellation = nextBinary(left);
      await actorMethods.handleFrame(source, {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'actor.method.cancel',
        invocationId: callerWins.invocationId,
        cancellationCorrelation: callerWins.cancellationCorrelation,
        reason: 'cancelled',
      }, new Uint8Array());
      expect(decodeActorMethodFrame(await callerCancellation).header).toMatchObject({
        type: 'actor.method.cancel',
        reason: 'cancelled',
      });
      await vi.advanceTimersByTimeAsync(100);
      await expect(
        registry.actorManager().registryStore()
          .actorInvocation(callerWins.invocationId)
      ).resolves.toMatchObject({
        state: 'cancelled',
        terminalReason: 'cancelled',
      });

      const deadlineWins = {
        ...invocation(actor, 'deadline-wins-caller-race'),
        deadline: {
          timeoutMs: 100,
          expiresAt: new Date(Date.now() + 100).toISOString(),
        },
      };
      const deadlineWinsOwnerInvoke = nextBinary(left);
      await actorMethods.handleFrame(source, deadlineWins, Buffer.from('[13]'));
      await deadlineWinsOwnerInvoke;
      const deadlineCancellation = nextBinary(left);
      await vi.advanceTimersByTimeAsync(100);
      expect(decodeActorMethodFrame(await deadlineCancellation).header).toMatchObject({
        type: 'actor.method.cancel',
        reason: 'deadlineExceeded',
      });
      await expect(actorMethods.handleFrame(source, {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'actor.method.cancel',
        invocationId: deadlineWins.invocationId,
        cancellationCorrelation: deadlineWins.cancellationCorrelation,
        reason: 'cancelled',
      }, new Uint8Array())).resolves.toBeUndefined();
      await expect(
        registry.actorManager().registryStore()
          .actorInvocation(deadlineWins.invocationId)
      ).resolves.toMatchObject({
        state: 'cancelled',
        terminalReason: 'deadlineExceeded',
      });
      await actorMethods.handleRuntimeDisconnect(source);
    } finally {
      vi.useRealTimers();
    }
  });

  it('claims every invocation on a disconnect before awaiting any ledger transition', async () => {
    const { actorMethods, registry, left, url } = await spawnHarness();
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    const owner = registry.runtimeConnection('runtime-a')!.ws;
    const warmup = invocation(actor, 'disconnect-batch-warmup');
    const warmupInvoke = nextBinary(left);
    await actorMethods.handleFrame(owner, warmup, Buffer.from('[0]'));
    await warmupInvoke;
    const warmupReturn = nextBinary(left);
    await actorMethods.handleFrame(owner, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: warmup.invocationId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }, new Uint8Array());
    await warmupReturn;
    await runtime(url, 'runtime-batch-caller');
    const caller = registry.runtimeConnection('runtime-batch-caller')!.ws;
    const invocationIds = [
      'disconnect-batch-first',
      'disconnect-batch-second',
      'disconnect-batch-third',
    ];
    for (const invocationId of invocationIds) {
      const ownerInvoke = nextBinary(left);
      await actorMethods.handleFrame(
        caller,
        invocation(actor, invocationId),
        Buffer.from('[14]')
      );
      await ownerInvoke;
    }

    const store = registry.actorManager().registryStore();
    const originalTransition = store.transitionActorInvocation.bind(store);
    let releaseFirst!: () => void;
    const firstBlocked = new Promise<void>((resolve) => {
      releaseFirst = resolve;
    });
    let reportFirstStarted!: () => void;
    const firstStarted = new Promise<void>((resolve) => {
      reportFirstStarted = resolve;
    });
    store.transitionActorInvocation = async (input) => {
      if (
        input.invocationId === invocationIds[0] &&
        input.nextState === 'cancelled'
      ) {
        reportFirstStarted();
        await firstBlocked;
      }
      return originalTransition(input);
    };

    const disconnecting = actorMethods.handleRuntimeDisconnect(caller);
    await firstStarted;
    for (const invocationId of invocationIds.slice(1)) {
      await expect(actorMethods.handleFrame(owner, {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'actor.method.return',
        invocationId,
        returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
      }, new Uint8Array())).resolves.toBeUndefined();
    }
    await waitForAsync(async () => {
      const states = await Promise.all(invocationIds.slice(1).map((invocationId) =>
        store.actorInvocation(invocationId)
      ));
      return states.every((entry) => entry?.state === 'cancelled');
    });
    releaseFirst();
    await disconnecting;
    for (const invocationId of invocationIds) {
      await expect(store.actorInvocation(invocationId)).resolves.toMatchObject({
        state: 'cancelled',
        terminalReason: 'caller Runtime disconnected',
      });
    }
  });

  it('reserves bounded correlation capacity before asynchronous admission', async () => {
    const admissionResolvers: Array<() => void> = [];
    let blockAdmissions = false;
    const { actorMethods, registry } = await spawnHarness({
      actorInvocationCorrelationCapacity: 4,
      onHasMethod: () => blockAdmissions
        ? new Promise<void>((resolve) => {
            admissionResolvers.push(resolve);
          })
        : undefined,
    });
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    const source = registry.runtimeConnection('runtime-a')!.ws;
    const warmup = invocation(actor, 'reservation-warmup');
    await actorMethods.handleFrame(source, warmup, Buffer.from('[0]'));
    await actorMethods.handleFrame(source, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: warmup.invocationId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }, new Uint8Array());
    blockAdmissions = true;
    const admissions = Array.from({ length: 4 }, (_, index) =>
      actorMethods.handleFrame(
        source,
        invocation(actor, `reserved-${index}`),
        Buffer.from(`[${index}]`)
      )
    );
    await waitFor(() => admissionResolvers.length === 4);

    await expect(actorMethods.handleFrame(
      source,
      invocation(actor, 'reserved-over-capacity'),
      Buffer.from('[99]')
    )).rejects.toThrow('Actor invocation correlation capacity exceeded');
    expect(admissionResolvers).toHaveLength(4);

    for (const resolve of admissionResolvers) resolve();
    await Promise.all(admissions);
    await actorMethods.handleRuntimeDisconnect(source);
  });

  it('applies one capacity across retained, pending, and reserved invocations', async () => {
    let blockAdmission = false;
    let releaseAdmission!: () => void;
    let reportAdmissionStarted!: () => void;
    const admissionStarted = new Promise<void>((resolve) => {
      reportAdmissionStarted = resolve;
    });
    const { actorMethods, registry, left } = await spawnHarness({
      actorInvocationCorrelationCapacity: 3,
      onHasMethod: () => blockAdmission
        ? new Promise<void>((resolve) => {
            releaseAdmission = resolve;
            reportAdmissionStarted();
          })
        : undefined,
    });
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    const source = registry.runtimeConnection('runtime-a')!.ws;

    const retained = invocation(actor, 'mixed-capacity-retained');
    const retainedOwnerInvoke = nextBinary(left);
    await actorMethods.handleFrame(source, retained, Buffer.from('[15]'));
    await retainedOwnerInvoke;
    const retainedCancellation = nextBinary(left);
    await actorMethods.handleFrame(source, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.cancel',
      invocationId: retained.invocationId,
      cancellationCorrelation: retained.cancellationCorrelation,
      reason: 'cancelled',
    }, new Uint8Array());
    await retainedCancellation;

    const pending = invocation(actor, 'mixed-capacity-pending');
    const pendingOwnerInvoke = nextBinary(left);
    await actorMethods.handleFrame(source, pending, Buffer.from('[16]'));
    await pendingOwnerInvoke;

    blockAdmission = true;
    const reserved = actorMethods.handleFrame(
      source,
      invocation(actor, 'mixed-capacity-reserved'),
      Buffer.from('[17]')
    );
    await admissionStarted;
    await expect(actorMethods.handleFrame(
      source,
      invocation(actor, 'mixed-capacity-overflow'),
      Buffer.from('[18]')
    )).rejects.toThrow('Actor invocation correlation capacity exceeded');
    await expect(actorMethods.handleFrame(source, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: retained.invocationId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }, new Uint8Array())).resolves.toBeUndefined();

    releaseAdmission();
    await reserved;
    await actorMethods.handleRuntimeDisconnect(source);
  });

  it('bounds high-frequency cancellation tombstones without evicting live correlation', async () => {
    const capacity = 64;
    let nextId = 0;
    const { actorMethods, registry, left } = await spawnHarness({
      actorInvocationCorrelationCapacity: capacity,
      id: () => `bounded-${nextId++}`,
    });
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    const owner = registry.runtimeConnection('runtime-a')!.ws;
    vi.useFakeTimers();
    try {
      let firstLateTerminal:
        | { invocationId: string; cancellationCorrelation: string }
        | undefined;
      const timerBaseline = vi.getTimerCount();
      for (let index = 0; index < capacity; index += 1) {
        const result = await actorMethods.submitSpawn(
          spawnSubmit({
            runtimeId: 'runtime-a',
            callerRequestId: `bounded-parent-${index}`,
            actor,
          }),
          Buffer.from(`[${index}]`),
          spawnContext(registry, 'runtime-a')
        );
        const dispatched = decodeActorOwnerInvokeFrame(await nextBinary(left));
        const cancellationCorrelation =
          dispatched.header.invoke.cancellationCorrelation;
        firstLateTerminal ??= {
          invocationId: result.requestId,
          cancellationCorrelation,
        };
        const echoedCancellation = nextBinary(left);
        await actorMethods.handleFrame(owner, {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'actor.method.cancel',
          invocationId: result.requestId,
          cancellationCorrelation,
          reason: 'cancelled',
        }, new Uint8Array());
        expect(decodeActorMethodFrame(await echoedCancellation).header).toMatchObject({
          type: 'actor.method.cancel',
          invocationId: result.requestId,
        });
        await vi.advanceTimersByTimeAsync(1);
      }
      expect(vi.getTimerCount()).toBeLessThanOrEqual(timerBaseline + 1);

      await expect(actorMethods.submitSpawn(
        spawnSubmit({
          runtimeId: 'runtime-a',
          callerRequestId: 'bounded-over-capacity',
          actor,
        }),
        Buffer.from('[65]'),
        spawnContext(registry, 'runtime-a')
      )).rejects.toThrow(
        'Actor invocation correlation capacity exceeded'
      );
      await expect(actorMethods.handleFrame(owner, {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'actor.method.return',
        invocationId: firstLateTerminal!.invocationId,
        returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
      }, new Uint8Array())).resolves.toBeUndefined();
      await expect(actorMethods.handleFrame(
        owner,
        invocation(actor, firstLateTerminal!.invocationId),
        Buffer.from('[67]')
      )).rejects.toThrow(
        `Actor invocation ${firstLateTerminal!.invocationId} is already tracked`
      );

      await vi.advanceTimersByTimeAsync(120_000);
      await expect(actorMethods.handleFrame(
        owner,
        invocation(actor, firstLateTerminal!.invocationId),
        Buffer.from('[68]')
      )).rejects.toThrow('Actor method admission rejected: InvocationAlreadyExists');
      const reused = await actorMethods.submitSpawn(
        spawnSubmit({
          runtimeId: 'runtime-a',
          callerRequestId: 'bounded-after-expiry',
          actor,
        }),
        Buffer.from('[66]'),
        spawnContext(registry, 'runtime-a')
      );
      await nextBinary(left);
      expect(reused.requestId).toContain('actor-spawn-bounded-');
    } finally {
      vi.useRealTimers();
    }
  });
});
