import { afterEach, describe, expect, it } from 'vitest';

import {
  encodeActorMethodFrame,
} from '../src/protocol/actorMethodProtocol.js';
import { decodeActorOwnerInvokeFrame } from '../src/protocol/actorOwnerProtocol.js';
import {
  decodeBinaryFrame,
  encodeRuntimeFrame,
} from '../src/protocol/envelope.js';
import {
  SERVICE_PROTOCOL,
  TEST_CAPABILITY,
  actorBootstrap,
  capabilityHarness,
  cleanupActorRoutingHarnesses,
  nextBinary,
  nextBinaryMessages,
  rootAuthority,
  spawnSubmit,
  terminalFrame,
} from './helpers/actorRoutingHarness.js';

afterEach(cleanupActorRoutingHarnesses);

const PACKAGE_BUILD =
  `skiff-package-build-v10:sha256:${'7'.repeat(64)}`;

describe('RuntimeEndpoint Actor message FIFO admission', () => {
  it('captures an active capability Actor parent before its following terminal frame', async () => {
    const { actorMethods, registry, left } = await capabilityHarness({
      dispatcher: true,
    });
    const actor = await registry.actorManager().getOrCreate(actorBootstrap(21));
    const parent = await actorMethods.submitSpawn(
      spawnSubmit({
        runtimeId: 'runtime-a',
        callerRequestId: 'fifo-root-spawn-first',
        actor,
      }),
      new Uint8Array(),
      capabilityContext(registry)
    );
    expect(
      decodeActorOwnerInvokeFrame(await nextBinary(left)).header.invoke
    ).toMatchObject({
      invocationId: parent.requestId,
      testCaseCapability: TEST_CAPABILITY,
      testCaseParentRequestId: 'fifo-root-spawn-first',
    });

    const childSubmit = {
      ...spawnSubmit({
        runtimeId: 'runtime-a',
        callerRequestId: parent.requestId,
        actor,
        serviceProtocolIdentity: SERVICE_PROTOCOL,
      }),
      buildId: PACKAGE_BUILD,
    };
    const messages = nextBinaryMessages(left, 2);
    left.send(encodeRuntimeFrame(childSubmit, Buffer.from('[1]')));
    left.send(encodeActorMethodFrame(
      terminalFrame('return', actor, parent.requestId)
    ));

    const [ownerBytes, responseBytes] = await messages;
    if (ownerBytes === undefined || responseBytes === undefined) {
      throw new Error('expected child owner invoke followed by spawn response');
    }
    const owner = decodeActorOwnerInvokeFrame(ownerBytes);
    expect(owner.header.invoke).toMatchObject({
      testCaseCapability: TEST_CAPABILITY,
      testCaseParentRequestId: parent.requestId,
    });
    const response = decodeBinaryFrame(responseBytes);
    expect(response.header).toMatchObject({
      type: 'spawn.submit.response',
      rpcId: childSubmit.rpcId,
      requestId: owner.header.invoke.invocationId,
      status: 'submitted',
    });

    left.send(encodeActorMethodFrame(
      terminalFrame('return', actor, owner.header.invoke.invocationId)
    ));
  });

  it('rejects a child spawn sent after its capability Actor parent terminal', async () => {
    const { actorMethods, registry, left } = await capabilityHarness({
      dispatcher: true,
    });
    const actor = await registry.actorManager().getOrCreate(actorBootstrap(22));
    const parent = await actorMethods.submitSpawn(
      spawnSubmit({
        runtimeId: 'runtime-a',
        callerRequestId: 'fifo-root-terminal-first',
        actor,
      }),
      new Uint8Array(),
      capabilityContext(registry)
    );
    await nextBinary(left);

    const childSubmit = {
      ...spawnSubmit({
        runtimeId: 'runtime-a',
        callerRequestId: parent.requestId,
        actor,
        serviceProtocolIdentity: SERVICE_PROTOCOL,
      }),
      buildId: PACKAGE_BUILD,
    };
    const responseMessage = nextBinary(left);
    left.send(encodeActorMethodFrame(
      terminalFrame('return', actor, parent.requestId)
    ));
    left.send(encodeRuntimeFrame(childSubmit, Buffer.from('[2]')));

    expect(decodeBinaryFrame(await responseMessage).header).toMatchObject({
      type: 'spawn.submit.error',
      rpcId: childSubmit.rpcId,
      error: {
        message: expect.stringContaining('active request or actor invocation'),
      },
    });
  });
});

function capabilityContext(registry: Awaited<
  ReturnType<typeof capabilityHarness>
>['registry']) {
  const connection = registry.runtimeConnection('runtime-a');
  if (connection === undefined) throw new Error('runtime-a is disconnected');
  return {
    originRuntimeId: 'runtime-a',
    originRuntimeConnection: connection.ws,
    testCaseCapability: TEST_CAPABILITY,
    authority: {
      ...rootAuthority('runtime-a', TEST_CAPABILITY),
      buildId: PACKAGE_BUILD,
    },
  };
}
