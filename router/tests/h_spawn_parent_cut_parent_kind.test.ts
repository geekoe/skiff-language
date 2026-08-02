import { afterEach, describe, expect, it } from 'vitest';

import {
  encodeActorMethodFrame,
} from '../src/protocol/actorMethodProtocol.js';
import { decodeActorOwnerInvokeFrame } from '../src/protocol/actorOwnerProtocol.js';
import {
  encodeBinaryFrame,
  decodeBinaryFrame,
  encodeRuntimeFrame,
  RUNTIME_FRAME_SCHEMA_VERSION,
  type SpawnCallerKind,
  type SpawnSubmitRequestFrameHeader,
} from '../src/protocol/envelope.js';
import { runtimeFrameHeaderFixtures } from '../src/protocol/runtimeProtocol.js';
import type {
  RuntimeDispatcher,
  RuntimeSpawnSubmitResult,
} from '../src/router/runtimeDispatcher.js';
import type { RuntimeRegistry } from '../src/router/runtimeRegistry.js';
import {
  ASSEMBLY,
  BUILD,
  SERVICE_ID,
  SERVICE_PROTOCOL,
  TEST_CAPABILITY,
  actorBootstrap,
  cleanupActorRoutingHarnesses,
  invocation,
  nextBinary,
  runtime,
  spawnHarness,
  spawnSubmit,
  testRoot,
} from './helpers/actorRoutingHarness.js';

afterEach(cleanupActorRoutingHarnesses);

type ActorRecord = Awaited<
  ReturnType<ReturnType<RuntimeRegistry['actorManager']>['getOrCreate']>
>;

function functionSpawnSubmit({
  runtimeId,
  callerKind,
  callerRequestId,
  generation = 1,
  serviceProtocolIdentity = SERVICE_PROTOCOL,
  missingActorMethod = false,
}: {
  runtimeId: string;
  callerKind: SpawnCallerKind;
  callerRequestId: string;
  generation?: number;
  serviceProtocolIdentity?: string;
  missingActorMethod?: boolean;
}): SpawnSubmitRequestFrameHeader {
  const fixture = runtimeFrameHeaderFixtures['spawn.submit.request'];
  return {
    ...fixture,
    rpcId: `spawn-rpc-${callerRequestId}`,
    runtimeId,
    callerKind,
    callerRequestId,
    targetKind: missingActorMethod ? 'actorMethod' : 'function',
    serviceId: SERVICE_ID,
    serviceVersion: '1.0.0',
    serviceProtocolIdentity,
    target: 'example.com/fn',
    spawnId: `spawn-${callerRequestId}`,
    buildId: BUILD,
    activationIdentity: {
      assemblyIdentity: ASSEMBLY,
      generation,
      runtimeReplicaId: runtimeId,
      deploymentRevision: 'rev-1',
    },
  } as unknown as SpawnSubmitRequestFrameHeader;
}

function legacySpawnSubmitWithoutCallerKind(
  callerRequestId: string
): Record<string, unknown> {
  const { callerKind: _callerKind, ...legacy } =
    runtimeFrameHeaderFixtures['spawn.submit.request'];
  return {
    ...legacy,
    rpcId: `spawn-rpc-legacy-${callerRequestId}`,
    runtimeId: 'runtime-a',
    callerRequestId,
    serviceId: SERVICE_ID,
    serviceProtocolIdentity: SERVICE_PROTOCOL,
    activationIdentity: {
      assemblyIdentity: ASSEMBLY,
      generation: 1,
      runtimeReplicaId: 'runtime-a',
      deploymentRevision: 'rev-1',
    },
  };
}

function responseEnd(requestId: string) {
  return {
    header: {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end' as const,
      requestId,
      payloadPresent: false,
      httpResponse: { status: 204, headers: [] },
    },
    payloadBytes: new Uint8Array(),
  };
}

async function seedRequestParent(
  dispatcher: RuntimeDispatcher,
  left: Awaited<ReturnType<typeof spawnHarness>>['left'],
  requestId: string
): Promise<void> {
  const root = dispatcher.dispatchAssemblyTestBinary(
    { header: testRoot(requestId, TEST_CAPABILITY), payloadBytes: new Uint8Array() },
    60_000
  );
  void root.catch(() => undefined);
  await nextBinary(left);
}

async function seedActorInvocationParent(
  actorMethods: Awaited<ReturnType<typeof spawnHarness>>['actorMethods'],
  left: Awaited<ReturnType<typeof spawnHarness>>['left'],
  actor: ActorRecord,
  invocationId: string
): Promise<void> {
  left.send(encodeActorMethodFrame(invocation(actor, invocationId)));
  const owner = decodeActorOwnerInvokeFrame(await nextBinary(left));
  expect(owner.header.invoke.invocationId).toBe(invocationId);
}

function assertSubmitted(result: RuntimeSpawnSubmitResult): void {
  expect(result.header.type).toBe('spawn.submit.response');
  if (result.header.type !== 'spawn.submit.response') return;
  expect(result.header.status).toBe('submitted');
}

describe('H-spawn-parent-cut production parent-kind selection', () => {
  it('resolve-function-parent-exact: request callerKind resolves the request pending only', async () => {
    const { registry, dispatcher, left } = await spawnHarness({ dispatcher: true });
    if (dispatcher === undefined) throw new Error('dispatcher harness missing');
    const serverWs = registry.runtimeConnection('runtime-a')!.ws;
    await seedRequestParent(dispatcher, left, 'parent-1');

    const response = await dispatcher.handleSpawnSubmit(
      serverWs,
      functionSpawnSubmit({
        runtimeId: 'runtime-a',
        callerKind: 'request',
        callerRequestId: 'parent-1',
      }),
      Buffer.from([1, 2])
    );
    assertSubmitted(response);
    if (response.header.type !== 'spawn.submit.response') return;

    const derived = decodeBinaryFrame(await nextBinary(left));
    expect(derived.header.type).toBe('request.start');
    expect((derived.header as { requestId: string }).requestId).toBe(
      response.header.requestId
    );
    dispatcher.resolveRequest(serverWs, responseEnd('parent-1'));
    dispatcher.resolveRequest(serverWs, responseEnd(response.header.requestId));
  });

  it('resolve-actor-invocation-parent-exact: actorInvocation callerKind resolves the actor parent only', async () => {
    const { registry, actorMethods, dispatcher, left } = await spawnHarness({
      dispatcher: true,
    });
    if (dispatcher === undefined) throw new Error('dispatcher harness missing');
    const actor = await registry.actorManager().getOrCreate(actorBootstrap());
    await seedActorInvocationParent(actorMethods, left, actor, 'inv-parent-1');
    const serverWs = registry.runtimeConnection('runtime-a')!.ws;

    const response = await dispatcher.handleSpawnSubmit(
      serverWs,
      spawnSubmit({
        runtimeId: 'runtime-a',
        callerKind: 'actorInvocation',
        callerRequestId: 'inv-parent-1',
        actor,
      }),
      Buffer.from('[1]')
    );
    assertSubmitted(response);
    if (response.header.type !== 'spawn.submit.response') return;

    const owner = decodeActorOwnerInvokeFrame(await nextBinary(left));
    expect(owner.header.invoke.invocationId).toBe(response.header.requestId);
    left.send(encodeActorMethodFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: response.header.requestId,
      returnEncodingVersion: 'skiff-actor-return-v1',
    }));
  });

  it('same-request-id-both-namespaces-no-collision: typed namespaces never collide', async () => {
    const { registry, actorMethods, dispatcher, left } = await spawnHarness({
      dispatcher: true,
    });
    if (dispatcher === undefined) throw new Error('dispatcher harness missing');
    const actor = await registry.actorManager().getOrCreate(actorBootstrap());
    await seedRequestParent(dispatcher, left, 'shared-1');
    await seedActorInvocationParent(actorMethods, left, actor, 'shared-1');
    const serverWs = registry.runtimeConnection('runtime-a')!.ws;

    const requestSubmit = await dispatcher.handleSpawnSubmit(
      serverWs,
      functionSpawnSubmit({
        runtimeId: 'runtime-a',
        callerKind: 'request',
        callerRequestId: 'shared-1',
      }),
      new Uint8Array()
    );
    assertSubmitted(requestSubmit);
    const derived = decodeBinaryFrame(await nextBinary(left));
    expect(derived.header.type).toBe('request.start');

    const actorSubmit = await dispatcher.handleSpawnSubmit(
      serverWs,
      spawnSubmit({
        runtimeId: 'runtime-a',
        callerKind: 'actorInvocation',
        callerRequestId: 'shared-1',
        actor,
      }),
      new Uint8Array()
    );
    assertSubmitted(actorSubmit);
    if (
      requestSubmit.header.type !== 'spawn.submit.response' ||
      actorSubmit.header.type !== 'spawn.submit.response'
    ) {
      return;
    }
    const owner = decodeActorOwnerInvokeFrame(await nextBinary(left));
    expect(owner.header.invoke.invocationId).toBe(actorSubmit.header.requestId);
    expect(owner.header.invoke.invocationId).not.toBe(requestSubmit.header.requestId);

    dispatcher.resolveRequest(serverWs, responseEnd('shared-1'));
    dispatcher.resolveRequest(serverWs, responseEnd(requestSubmit.header.requestId));
    left.send(encodeActorMethodFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: actorSubmit.header.requestId,
      returnEncodingVersion: 'skiff-actor-return-v1',
    }));
  });

  it('missing-caller-kind-legacy-cut-rejected: the old shape is a protocol terminal', async () => {
    const { left } = await spawnHarness({ dispatcher: true });
    const closed = new Promise<number | undefined>((resolve) => {
      left.once('close', (code) => resolve(code));
    });
    left.send(encodeBinaryFrame(
      legacySpawnSubmitWithoutCallerKind('legacy-parent-1'),
      Buffer.from([1])
    ));
    const code = await Promise.race([
      closed,
      new Promise<number | undefined>((resolve) =>
        setTimeout(() => resolve(undefined), 2_000)
      ),
    ]);
    expect(code).toBe(1008);
  });

  it('parent-terminal-before-submit-rejected: a terminal request parent cannot accept', async () => {
    const { registry, dispatcher, left } = await spawnHarness({ dispatcher: true });
    if (dispatcher === undefined) throw new Error('dispatcher harness missing');
    const serverWs = registry.runtimeConnection('runtime-a')!.ws;
    await seedRequestParent(dispatcher, left, 'parent-terminal-1');
    dispatcher.resolveRequest(serverWs, responseEnd('parent-terminal-1'));

    const response = await dispatcher.handleSpawnSubmit(
      serverWs,
      functionSpawnSubmit({
        runtimeId: 'runtime-a',
        callerKind: 'request',
        callerRequestId: 'parent-terminal-1',
      }),
      new Uint8Array()
    );
    expect(response.header).toMatchObject({
      type: 'spawn.submit.error',
      error: {
        message:
          'spawn callerRequestId does not identify an active request parent on the same runtime connection',
      },
    });
  });

  it('parent-replaced-before-submit-rejected: a replaced actor parent connection cannot accept', async () => {
    const { registry, actorMethods, dispatcher, left, url } = await spawnHarness({
      dispatcher: true,
    });
    if (dispatcher === undefined) throw new Error('dispatcher harness missing');
    const actor = await registry.actorManager().getOrCreate(actorBootstrap());
    await seedActorInvocationParent(actorMethods, left, actor, 'inv-replaced-1');
    const closed = new Promise<void>((resolve) => left.once('close', () => resolve()));
    left.close();
    await closed;
    await new Promise<void>((resolve) => setTimeout(resolve, 50));
    const replacement = await runtime(url, 'runtime-a', SERVICE_ID, registry);
    const replacementWs = registry.runtimeConnection('runtime-a')!.ws;

    const response = await dispatcher.handleSpawnSubmit(
      replacementWs,
      spawnSubmit({
        runtimeId: 'runtime-a',
        callerKind: 'actorInvocation',
        callerRequestId: 'inv-replaced-1',
        actor,
      }),
      new Uint8Array()
    );
    expect(response.header.type).toBe('spawn.submit.error');
    expect(replacement.readyState).toBe(1);
  });

  it('parent-connection-mismatch-rejected: a submit on another runtime connection is rejected', async () => {
    const { registry, dispatcher, left } = await spawnHarness({
      dispatcher: true,
      secondRuntime: true,
    });
    if (dispatcher === undefined) throw new Error('dispatcher harness missing');
    await seedRequestParent(dispatcher, left, 'parent-conn-1');
    const otherWs = registry.runtimeConnection('runtime-b')!.ws;

    const response = await dispatcher.handleSpawnSubmit(
      otherWs,
      functionSpawnSubmit({
        runtimeId: 'runtime-a',
        callerKind: 'request',
        callerRequestId: 'parent-conn-1',
      }),
      new Uint8Array()
    );
    expect(response.header.type).toBe('spawn.submit.error');
  });

  it('authority-mismatch-rejected: drifted authority facts fail closed', async () => {
    const { registry, actorMethods, dispatcher, left } = await spawnHarness({
      dispatcher: true,
    });
    if (dispatcher === undefined) throw new Error('dispatcher harness missing');
    const actor = await registry.actorManager().getOrCreate(actorBootstrap());
    await seedActorInvocationParent(actorMethods, left, actor, 'inv-authority-1');
    const serverWs = registry.runtimeConnection('runtime-a')!.ws;

    const response = await dispatcher.handleSpawnSubmit(
      serverWs,
      spawnSubmit({
        runtimeId: 'runtime-a',
        callerKind: 'actorInvocation',
        callerRequestId: 'inv-authority-1',
        actor,
        generation: 2,
      }),
      new Uint8Array()
    );
    expect(response.header).toMatchObject({
      type: 'spawn.submit.error',
      error: {
        message:
          'spawn submit owner facts must exactly match its authenticated parent',
      },
    });
  });

  it('accepted-spawn-outlives-parent-terminal: acceptance is decoupled from the parent', async () => {
    const { registry, dispatcher, left } = await spawnHarness({ dispatcher: true });
    if (dispatcher === undefined) throw new Error('dispatcher harness missing');
    const serverWs = registry.runtimeConnection('runtime-a')!.ws;
    await seedRequestParent(dispatcher, left, 'parent-outlives-1');

    const accepted = await dispatcher.handleSpawnSubmit(
      serverWs,
      functionSpawnSubmit({
        runtimeId: 'runtime-a',
        callerKind: 'request',
        callerRequestId: 'parent-outlives-1',
      }),
      new Uint8Array()
    );
    assertSubmitted(accepted);
    if (accepted.header.type !== 'spawn.submit.response') return;
    const derived = decodeBinaryFrame(await nextBinary(left));
    expect(derived.header.type).toBe('request.start');

    // Parent terminal after acceptance does not cancel the accepted spawn.
    dispatcher.resolveRequest(serverWs, responseEnd('parent-outlives-1'));
    const rejected = await dispatcher.handleSpawnSubmit(
      serverWs,
      functionSpawnSubmit({
        runtimeId: 'runtime-a',
        callerKind: 'request',
        callerRequestId: 'parent-outlives-1',
      }),
      new Uint8Array()
    );
    expect(rejected.header.type).toBe('spawn.submit.error');

    dispatcher.resolveRequest(serverWs, responseEnd(accepted.header.requestId));
  });

  it('target-kind-mismatch-rejected: actorMethod target without metadata fails closed', async () => {
    const { registry, dispatcher, left } = await spawnHarness({ dispatcher: true });
    if (dispatcher === undefined) throw new Error('dispatcher harness missing');
    const serverWs = registry.runtimeConnection('runtime-a')!.ws;
    await seedRequestParent(dispatcher, left, 'parent-mismatch-1');

    const response = await dispatcher.handleSpawnSubmit(
      serverWs,
      functionSpawnSubmit({
        runtimeId: 'runtime-a',
        callerKind: 'request',
        callerRequestId: 'parent-mismatch-1',
        missingActorMethod: true,
      }),
      new Uint8Array()
    );
    expect(response.header.type).toBe('spawn.submit.error');
  });
});
