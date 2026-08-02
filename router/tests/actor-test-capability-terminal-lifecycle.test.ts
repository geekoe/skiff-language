import { afterEach, describe, expect, it } from 'vitest';

import {
  decodeActorMethodFrame,
  encodeActorMethodFrame,
} from '../src/protocol/actorMethodProtocol.js';
import {
  SERVICE_PROTOCOL,
  TEST_CAPABILITY,
  actorBootstrap,
  capabilityHarness as spawnHarness,
  cleanupActorRoutingHarnesses,
  delay,
  invocation,
  nextBinary,
  spawnSubmit,
  terminalFrame,
  terminalLedgerState,
  testRoot,
  waitForAsync,
} from './helpers/actorRoutingHarness.js';

afterEach(cleanupActorRoutingHarnesses);

describe('actor test capability terminal lifecycle', () => {
  for (const terminalKind of ['return', 'error', 'cancel'] as const) {
    it(`removes capability lineage before a blocked ${terminalKind} ledger transition`, async () => {
      const {
        registry,
        dispatcher,
        left,
        issuedIds,
      } = await spawnHarness({ dispatcher: true });
      if (dispatcher === undefined) throw new Error('dispatcher harness missing');
      const actor = await registry.actorManager().getOrCreate(actorBootstrap(
        terminalKind === 'return' ? 16 : terminalKind === 'error' ? 17 : 18
      ));
      const rootId = `terminal-${terminalKind}-root`;
      const parentId = `terminal-${terminalKind}-parent`;
      const root = dispatcher.dispatchAssemblyTestBinary(
        {
          header: testRoot(rootId, TEST_CAPABILITY),
          payloadBytes: new Uint8Array(),
        },
        60_000
      );
      void root.catch(() => undefined);
      await nextBinary(left);
      left.send(encodeActorMethodFrame(
        invocation(actor, parentId, {
          testCaseCapability: TEST_CAPABILITY,
          testCaseParentRequestId: rootId,
        })
      ));
      await nextBinary(left);

      const store = registry.actorManager().registryStore();
      const originalTransition = store.transitionActorInvocation.bind(store);
      let markStarted!: () => void;
      const transitionStarted = new Promise<void>((resolve) => {
        markStarted = resolve;
      });
      let releaseTransition!: () => void;
      const transitionGate = new Promise<void>((resolve) => {
        releaseTransition = resolve;
      });
      let blocked = false;
      store.transitionActorInvocation = async (input) => {
        if (
          !blocked &&
          input.invocationId === parentId &&
          input.nextState === terminalLedgerState(terminalKind)
        ) {
          blocked = true;
          markStarted();
          await transitionGate;
        }
        return originalTransition(input);
      };

      const cancelEcho = terminalKind === 'cancel' ? nextBinary(left) : undefined;
      left.send(encodeActorMethodFrame(
        terminalFrame(terminalKind, actor, parentId)
      ));
      await transitionStarted;
      if (cancelEcho !== undefined) {
        expect(decodeActorMethodFrame(await cancelEcho).header).toMatchObject({
          type: 'actor.method.cancel',
          invocationId: parentId,
        });
      }

      let unexpectedOwnerMessages = 0;
      const countUnexpected = () => unexpectedOwnerMessages += 1;
      left.on('message', countUnexpected);
      const issuedBefore = issuedIds.length;
      const rejected = await dispatcher.handleSpawnSubmit(
        registry.runtimeConnection('runtime-a')!.ws,
        spawnSubmit({
          runtimeId: 'runtime-a',
          callerRequestId: parentId,
          actor,
          serviceProtocolIdentity: SERVICE_PROTOCOL,
        }),
        new Uint8Array()
      );
      expect(rejected.header).toMatchObject({
        type: 'spawn.submit.error',
        error: { message: expect.stringContaining('active request or actor') },
      });
      expect(issuedIds).toHaveLength(issuedBefore);
      await delay(20);
      expect(unexpectedOwnerMessages).toBe(0);
      left.off('message', countUnexpected);

      releaseTransition();
      await waitForAsync(async () =>
        (await store.actorInvocation(parentId))?.state ===
          terminalLedgerState(terminalKind)
      );
    });
  }
});
