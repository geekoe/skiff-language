import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  DEFAULT_JSON_RPC_20_TEXT_LIMITS,
  JsonRpc20TextProfile,
  type OpaquePayload,
  type ProfileLimits
} from '../src/protocol/jsonRpc20TextProfile.js';
import {
  WebSocketRequestBroker,
  type BrokerRuntimeResponse,
  type BrokerRuntimeSource,
  type InboundDispatchAction,
  type InboundDispatchResult
} from '../src/router/webSocketRequestBroker.js';

afterEach(() => {
  vi.useRealTimers();
});

describe('WebSocketRequestBroker outbound core', () => {
  it('completes out-of-order responses on the exact captured runtime source', async () => {
    const harness = createHarness();
    const runtimeA = createRuntime('session-a');
    const runtimeB = createRuntime('session-b');

    harness.request(runtimeA, 'runtime-a');
    harness.request(runtimeB, 'runtime-b');
    expect(harness.peer.writes).toHaveLength(2);
    const [peerA, peerB] = harness.peer.writes.map(outboundId);

    harness.broker.handlePeerText(
      harness.generation,
      `{"jsonrpc":"2.0","id":${JSON.stringify(peerB)},"result":{"for":"b"}}`
    );
    harness.broker.handlePeerText(
      harness.generation,
      `{"jsonrpc":"2.0","id":${JSON.stringify(peerA)},"result":{"for":"a"}}`
    );
    await flush();

    expect(runtimeA.responses).toEqual([
      expect.objectContaining({
        requestId: 'runtime-a',
        outcome: 'success',
        payloadText: '{"for":"a"}'
      })
    ]);
    expect(runtimeB.responses).toEqual([
      expect.objectContaining({
        requestId: 'runtime-b',
        outcome: 'success',
        payloadText: '{"for":"b"}'
      })
    ]);
    expectNoActive(harness.broker);
  });

  it('preserves a peer remote error and optional opaque data', async () => {
    const harness = createHarness();
    const runtime = createRuntime('session-a');
    harness.request(runtime, 'runtime-a');
    const id = outboundId(harness.peer.writes[0]!);

    harness.broker.handlePeerText(
      harness.generation,
      `{"jsonrpc":"2.0","id":${JSON.stringify(id)},"error":` +
        '{"code":-32603,"message":"peer failed","data":{"n":9007199254740993}}}'
    );
    await flush();

    expect(runtime.responses).toEqual([
      expect.objectContaining({
        requestId: 'runtime-a',
        outcome: 'remote',
        remote: {
          code: -32603,
          message: 'peer failed',
          dataPresent: true
        },
        payloadText: '{"n":9007199254740993}'
      })
    ]);
    expectNoActive(harness.broker);
  });

  it('treats a batch containing an active id as one invalid request without table lookup', async () => {
    const harness = createHarness();
    const runtime = createRuntime('session-a');
    harness.request(runtime, 'runtime-a');
    const id = outboundId(harness.peer.writes[0]!);

    harness.broker.handlePeerText(
      harness.generation,
      `[{"jsonrpc":"2.0","id":${JSON.stringify(id)},"result":true}]`
    );
    expect(runtime.responses).toEqual([]);
    expect(harness.peer.writes.at(-1)).toBe(
      '{"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Invalid Request"}}'
    );
    expect(harness.broker.debugSnapshot().outboundPeerEntries).toBe(1);

    harness.broker.handlePeerText(
      harness.generation,
      `{"jsonrpc":"2.0","id":${JSON.stringify(id)},"result":true}`
    );
    await flush();
    expect(runtime.responses).toHaveLength(1);
    expectNoActive(harness.broker);
  });

  it('runtime cancel detaches before a best-effort peer cancel and sends no response', async () => {
    const harness = createHarness();
    const runtime = createRuntime('session-a');
    harness.request(runtime, 'runtime-a');
    const id = outboundId(harness.peer.writes[0]!);

    expect(harness.broker.handleRuntimeCancel(runtime, 'runtime-a')).toBe(true);
    await flush();

    expect(runtime.responses).toEqual([]);
    expect(harness.peer.writes.at(-1)).toBe(
      `{"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":${JSON.stringify(id)}}}`
    );
    expectNoActive(harness.broker);
  });

  it('deadline wins once, cancels the peer, and reports deadlineExceeded', async () => {
    vi.useFakeTimers();
    const harness = createHarness();
    const runtime = createRuntime('session-a');
    harness.request(runtime, 'runtime-a', {
      deadlineAtMs: Date.now() + 50
    });
    const id = outboundId(harness.peer.writes[0]!);

    await vi.advanceTimersByTimeAsync(50);

    expect(runtime.responses).toEqual([
      expect.objectContaining({
        requestId: 'runtime-a',
        outcome: 'deadlineExceeded'
      })
    ]);
    expect(harness.peer.writes.at(-1)).toContain('$/cancelRequest');
    harness.broker.handlePeerText(
      harness.generation,
      `{"jsonrpc":"2.0","id":${JSON.stringify(id)},"result":null}`
    );
    await flush();
    expect(runtime.responses).toHaveLength(1);
    expectNoActive(harness.broker);
  });

  it('writer failure settles only the exact request as transportUnavailable', async () => {
    const harness = createHarness({
      writeText(frame) {
        harness.peer.writes.push(frame);
        throw new Error('socket write failed');
      }
    });
    const runtime = createRuntime('session-a');

    harness.request(runtime, 'runtime-a');
    await flush();

    expect(runtime.responses).toEqual([
      expect.objectContaining({
        requestId: 'runtime-a',
        outcome: 'transportUnavailable'
      })
    ]);
    expectNoActive(harness.broker);
  });

  it('runtime disconnect cancels only that captured session without a response', async () => {
    const harness = createHarness();
    const runtimeA = createRuntime('session-a');
    const runtimeB = createRuntime('session-b');
    harness.request(runtimeA, 'runtime-a');
    harness.request(runtimeB, 'runtime-b');
    const peerB = outboundId(harness.peer.writes[1]!);

    expect(harness.broker.handleRuntimeDisconnect(runtimeA)).toBe(1);
    harness.broker.handlePeerText(
      harness.generation,
      `{"jsonrpc":"2.0","id":${JSON.stringify(peerB)},"result":true}`
    );
    await flush();

    expect(runtimeA.responses).toEqual([]);
    expect(runtimeB.responses).toEqual([
      expect.objectContaining({
        requestId: 'runtime-b',
        outcome: 'success'
      })
    ]);
    expectNoActive(harness.broker);
  });

  it('peer disconnect settles outbound and aborts inbound without socket writes', async () => {
    const pending = deferred<InboundDispatchResult>();
    const harness = createHarness({
      dispatchInbound: () => pending.promise
    });
    const runtime = createRuntime('session-a');
    harness.request(runtime, 'runtime-a');
    harness.broker.handlePeerText(
      harness.generation,
      '{"jsonrpc":"2.0","id":"peer-a","method":"status.get","params":{}}'
    );
    const writesBeforeDisconnect = harness.peer.writes.length;

    harness.broker.handlePeerDisconnect(harness.generation);
    await flush();

    expect(runtime.responses).toEqual([
      expect.objectContaining({ outcome: 'transportUnavailable' })
    ]);
    expect(harness.dispatches[0]!.signal.aborted).toBe(true);
    pending.resolve({
      kind: 'success',
      result: opaqueResult(harness.profile, '{"late":true}')
    });
    await flush();
    expect(harness.peer.writes).toHaveLength(writesBeforeDisconnect);
    expectNoActive(harness.broker);
  });

  it('binary/protocol close reports protocolError and closes with 1003', async () => {
    const harness = createHarness();
    const runtime = createRuntime('session-a');
    harness.request(runtime, 'runtime-a');

    harness.broker.handlePeerBinary(harness.generation);
    await flush();

    expect(harness.peer.closes).toEqual([
      { code: 1003, reason: 'binary RPC frames are not supported' }
    ]);
    expect(runtime.responses).toEqual([
      expect.objectContaining({ outcome: 'protocolError' })
    ]);
    expectNoActive(harness.broker);
  });

  it('oversized peer text closes 1009 and settles pending as protocolError', async () => {
    const harness = createHarness({
      profileLimits: {
        ...DEFAULT_JSON_RPC_20_TEXT_LIMITS,
        maxTextBytes: 200
      }
    });
    const runtime = createRuntime('session-a');
    harness.request(runtime, 'runtime-a');
    expect(harness.peer.writes).toHaveLength(1);

    harness.broker.handlePeerText(harness.generation, 'x'.repeat(201));
    await flush();

    expect(harness.peer.closes).toEqual([
      {
        code: 1009,
        reason: 'JSON-RPC text frame exceeds profile limits'
      }
    ]);
    expect(runtime.responses).toEqual([
      expect.objectContaining({ outcome: 'protocolError' })
    ]);
    expectNoActive(harness.broker);
  });

  it('keeps an old generation pending on its captured writer after replacement', async () => {
    const harness = createHarness();
    const runtime = createRuntime('session-a');
    harness.request(runtime, 'runtime-a');
    const oldId = outboundId(harness.peer.writes[0]!);
    const replacementPeer = createPeer();
    const replacement = harness.broker.attachGeneration({
      connectionId: harness.generation.connectionId,
      socketGeneration: 'generation-b',
      serviceId: 'example/chat',
      websocketEntryId: 'entry-a',
      ownerToken: {},
      profile: 'jsonrpc-2.0-text',
      outboundIdPrefix: 'generation-b',
      writer: replacementPeer
    });

    harness.broker.handlePeerText(
      replacement,
      `{"jsonrpc":"2.0","id":${JSON.stringify(oldId)},"result":"wrong-generation"}`
    );
    expect(runtime.responses).toEqual([]);
    expect(replacementPeer.closes).toEqual([
      { code: 1002, reason: 'unknown JSON-RPC response id' }
    ]);

    harness.broker.handlePeerText(
      harness.generation,
      `{"jsonrpc":"2.0","id":${JSON.stringify(oldId)},"result":"old"}`
    );
    await flush();

    expect(runtime.responses).toEqual([
      expect.objectContaining({ outcome: 'success', payloadText: '"old"' })
    ]);
    expect(replacementPeer.writes).toEqual([]);
    expect(harness.broker.debugSnapshot().generationCount).toBe(1);
    expectNoActive(harness.broker);
  });

  it('rejects connection ownership mismatch before peer write', async () => {
    const harness = createHarness();
    const runtime = createRuntime('session-a');

    harness.broker.handleRuntimeRequest(harness.generation, {
      source: runtime,
      requestId: 'runtime-a',
      serviceId: 'example/other',
      websocketEntryId: 'entry-a',
      ownerToken: harness.ownerToken,
      profile: 'jsonrpc-2.0-text',
      method: 'status.get',
      payloadBytes: Buffer.from('{}')
    });
    await flush();

    expect(harness.peer.writes).toEqual([]);
    expect(runtime.responses).toEqual([
      expect.objectContaining({ outcome: 'connectionUnavailable' })
    ]);
    expectNoActive(harness.broker);
  });

  it('rejects outbound capacity before allocating or writing a second peer id', async () => {
    const harness = createHarness({
      outboundGlobalCapacity: 1,
      outboundPerGenerationCapacity: 1
    });
    const runtime = createRuntime('session-a');
    harness.request(runtime, 'runtime-a');
    harness.request(runtime, 'runtime-b');
    await flush();

    expect(harness.peer.writes).toHaveLength(1);
    expect(runtime.responses).toEqual([
      expect.objectContaining({
        requestId: 'runtime-b',
        outcome: 'resourceLimit'
      })
    ]);
    harness.broker.handlePeerDisconnect(harness.generation);
    expectNoActive(harness.broker);
  });
});

describe('WebSocketRequestBroker inbound core', () => {
  it('dispatches request params opaquely and writes one successful result', async () => {
    const harness = createHarness({
      dispatchInbound(action) {
        expect(harness.profile.opaqueJsonText(action.params)).toBe(
          '{"n":9007199254740993}'
        );
        return {
          kind: 'success',
          result: opaqueResult(harness.profile, '{"ok":true}')
        };
      }
    });

    harness.broker.handlePeerText(
      harness.generation,
      '{"jsonrpc":"2.0","id":7,"method":"status.get","params":{"n":9007199254740993}}'
    );
    await flush();

    expect(harness.dispatches).toHaveLength(1);
    expect(harness.dispatches[0]).toMatchObject({
      profile: 'jsonrpc-2.0-text',
      connectionId: 'connection-a',
      socketGeneration: 'generation-a',
      peerId: { kind: 'safeInteger', value: 7 },
      method: 'status.get'
    });
    expect(harness.peer.writes).toEqual([
      '{"jsonrpc":"2.0","id":7,"result":{"ok":true}}'
    ]);
    expectNoActive(harness.broker);
  });

  it('emits an ignored notification action without dispatch or terminal write', () => {
    const notifications: unknown[] = [];
    const harness = createHarness({
      observeNotification: (action) => notifications.push(action)
    });

    harness.broker.handlePeerText(
      harness.generation,
      '{"jsonrpc":"2.0","method":"telemetry.observe","params":{"ok":true}}'
    );

    expect(notifications).toEqual([
      expect.objectContaining({
        profile: 'jsonrpc-2.0-text',
        connectionId: 'connection-a',
        method: 'telemetry.observe'
      })
    ]);
    expect(harness.dispatches).toEqual([]);
    expect(harness.peer.writes).toEqual([]);
    expectNoActive(harness.broker);
  });

  it('keeps an id-bearing reserved cancel spelling out of user dispatch', () => {
    const harness = createHarness();

    harness.broker.handlePeerText(
      harness.generation,
      '{"jsonrpc":"2.0","id":"peer-a","method":"$/cancelRequest","params":{"id":"other"}}'
    );

    expect(harness.dispatches).toEqual([]);
    expect(harness.peer.writes).toEqual([
      '{"jsonrpc":"2.0","id":"peer-a","error":{"code":-32601,"message":"Method not found"}}'
    ]);
    expectNoActive(harness.broker);
  });

  it('peer cancel wins against completion, aborts, and writes exactly one cancellation', async () => {
    const completion = deferred<InboundDispatchResult>();
    const harness = createHarness({
      dispatchInbound: () => completion.promise
    });
    harness.broker.handlePeerText(
      harness.generation,
      '{"jsonrpc":"2.0","id":"peer-a","method":"status.get","params":{}}'
    );

    harness.broker.handlePeerText(
      harness.generation,
      '{"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":"peer-a"}}'
    );
    harness.broker.handlePeerText(
      harness.generation,
      '{"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":"peer-a"}}'
    );
    expect(harness.dispatches[0]!.signal.aborted).toBe(true);
    completion.resolve({
      kind: 'success',
      result: opaqueResult(harness.profile, '{"late":true}')
    });
    await flush();

    expect(harness.peer.writes).toEqual([
      '{"jsonrpc":"2.0","id":"peer-a","error":{"code":-32800,"message":"Request cancelled"}}'
    ]);
    expectNoActive(harness.broker);
  });

  it('inbound deadline wins against a late dispatcher completion', async () => {
    vi.useFakeTimers();
    const completion = deferred<InboundDispatchResult>();
    const harness = createHarness({
      dispatchInbound: () => completion.promise,
      inboundTimeoutMs: 25
    });
    harness.broker.handlePeerText(
      harness.generation,
      '{"jsonrpc":"2.0","id":"peer-a","method":"status.get","params":{}}'
    );

    await vi.advanceTimersByTimeAsync(25);
    completion.resolve({
      kind: 'success',
      result: opaqueResult(harness.profile, 'true')
    });
    await flush();

    expect(harness.dispatches[0]!.signal.aborted).toBe(true);
    expect(harness.peer.writes).toEqual([
      '{"jsonrpc":"2.0","id":"peer-a","error":{"code":-32001,"message":"Request timed out"}}'
    ]);
    expectNoActive(harness.broker);
  });

  it('active and tombstoned duplicate ids close 1002 and abort the generation', async () => {
    const completion = deferred<InboundDispatchResult>();
    const harness = createHarness({
      dispatchInbound: () => completion.promise
    });
    const frame =
      '{"jsonrpc":"2.0","id":"peer-a","method":"status.get","params":{}}';
    harness.broker.handlePeerText(harness.generation, frame);
    harness.broker.handlePeerText(harness.generation, frame);
    await flush();

    expect(harness.peer.closes).toEqual([
      { code: 1002, reason: 'duplicate JSON-RPC request id' }
    ]);
    expect(harness.dispatches[0]!.signal.aborted).toBe(true);
    expect(harness.peer.writes).toEqual([]);
    expectNoActive(harness.broker);
  });

  it('capacity writes server busy after tombstoning the rejected id', async () => {
    const first = deferred<InboundDispatchResult>();
    const harness = createHarness({
      dispatchInbound: () => first.promise,
      inboundGlobalCapacity: 1,
      inboundPerGenerationCapacity: 1
    });
    harness.broker.handlePeerText(
      harness.generation,
      '{"jsonrpc":"2.0","id":"peer-a","method":"status.get","params":{}}'
    );
    harness.broker.handlePeerText(
      harness.generation,
      '{"jsonrpc":"2.0","id":"peer-b","method":"status.get","params":{}}'
    );

    expect(harness.peer.writes).toEqual([
      '{"jsonrpc":"2.0","id":"peer-b","error":{"code":-32000,"message":"Server busy"}}'
    ]);
    harness.broker.handlePeerText(
      harness.generation,
      '{"jsonrpc":"2.0","id":"peer-b","method":"status.get","params":{}}'
    );
    expect(harness.peer.closes).toEqual([
      { code: 1002, reason: 'duplicate JSON-RPC request id' }
    ]);
    await flush();
    expectNoActive(harness.broker);
  });

  it('maps dispatcher outcomes to fixed errors without exposing details', async () => {
    const outcomes: InboundDispatchResult[] = [
      { kind: 'invalidParams' },
      { kind: 'internalError' },
      { kind: 'deadlineExceeded' },
      { kind: 'runtimeUnavailable' }
    ];
    const harness = createHarness({
      dispatchInbound: () => outcomes.shift()!
    });

    for (const [index, expectedCode] of [-32602, -32603, -32001, -32603].entries()) {
      harness.broker.handlePeerText(
        harness.generation,
        `{"jsonrpc":"2.0","id":"peer-${index}","method":"status.get","params":{}}`
      );
      await flush();
      expect(JSON.parse(harness.peer.writes[index]!).error.code).toBe(
        expectedCode
      );
    }
    expectNoActive(harness.broker);
  });

  it('keeps the same peer id independent across outbound and inbound tables', async () => {
    const inbound = deferred<InboundDispatchResult>();
    const harness = createHarness({
      dispatchInbound: () => inbound.promise
    });
    const runtime = createRuntime('session-a');
    harness.request(runtime, 'runtime-a');
    const sharedId = outboundId(harness.peer.writes[0]!);
    harness.broker.handlePeerText(
      harness.generation,
      `{"jsonrpc":"2.0","id":${JSON.stringify(sharedId)},"method":"status.get","params":{}}`
    );
    harness.broker.handlePeerText(
      harness.generation,
      `{"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":${JSON.stringify(sharedId)}}}`
    );
    harness.broker.handlePeerText(
      harness.generation,
      `{"jsonrpc":"2.0","id":${JSON.stringify(sharedId)},"result":{"outbound":true}}`
    );
    await flush();

    expect(runtime.responses).toEqual([
      expect.objectContaining({ outcome: 'success' })
    ]);
    expect(harness.peer.writes.filter((frame) =>
      frame.includes('"Request cancelled"')
    )).toHaveLength(1);
    expectNoActive(harness.broker);
  });

  it('tombstone FIFO eviction permits id reuse while old execution tokens stay fenced', async () => {
    const old = deferred<InboundDispatchResult>();
    const middle = deferred<InboundDispatchResult>();
    const current = deferred<InboundDispatchResult>();
    const completions = [old, middle, current];
    const harness = createHarness({
      inboundTombstoneCapacity: 1,
      dispatchInbound: () => completions.shift()!.promise
    });
    const request = (id: string) => harness.broker.handlePeerText(
      harness.generation,
      `{"jsonrpc":"2.0","id":${JSON.stringify(id)},"method":"status.get","params":{}}`
    );
    const cancel = (id: string) => harness.broker.handlePeerText(
      harness.generation,
      `{"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":${JSON.stringify(id)}}}`
    );

    request('reused');
    cancel('reused');
    request('evictor');
    cancel('evictor');
    request('reused');
    const writesBeforeLate = harness.peer.writes.length;

    old.resolve({
      kind: 'success',
      result: opaqueResult(harness.profile, '{"old":true}')
    });
    await flush();
    expect(harness.peer.writes).toHaveLength(writesBeforeLate);

    current.resolve({
      kind: 'success',
      result: opaqueResult(harness.profile, '{"current":true}')
    });
    await flush();
    expect(harness.peer.writes.at(-1)).toBe(
      '{"jsonrpc":"2.0","id":"reused","result":{"current":true}}'
    );
    expectNoActive(harness.broker);
  });

  it('lazy TTL sweep permits an inbound id only after its tombstone expires', async () => {
    vi.useFakeTimers();
    const harness = createHarness({ inboundTombstoneTtlMs: 20 });
    const frame =
      '{"jsonrpc":"2.0","id":"ttl-id","method":"status.get","params":{}}';

    harness.broker.handlePeerText(harness.generation, frame);
    await flush();
    expect(harness.dispatches).toHaveLength(1);

    await vi.advanceTimersByTimeAsync(20);
    harness.broker.handlePeerText(harness.generation, frame);
    await flush();

    expect(harness.dispatches).toHaveLength(2);
    expect(harness.peer.writes).toHaveLength(2);
    expectNoActive(harness.broker);
  });

  it('silently drops a late outbound response while tombstoned and closes after eviction', async () => {
    const harness = createHarness({ outboundTombstoneCapacity: 1 });
    const runtime = createRuntime('session-a');
    harness.request(runtime, 'runtime-a');
    const oldId = outboundId(harness.peer.writes[0]!);
    harness.broker.handlePeerText(
      harness.generation,
      `{"jsonrpc":"2.0","id":${JSON.stringify(oldId)},"result":1}`
    );
    await flush();
    harness.broker.handlePeerText(
      harness.generation,
      `{"jsonrpc":"2.0","id":${JSON.stringify(oldId)},"result":2}`
    );
    expect(harness.peer.closes).toEqual([]);

    harness.request(runtime, 'runtime-b');
    const newId = outboundId(harness.peer.writes.at(-1)!);
    harness.broker.handlePeerText(
      harness.generation,
      `{"jsonrpc":"2.0","id":${JSON.stringify(newId)},"result":3}`
    );
    await flush();
    harness.broker.handlePeerText(
      harness.generation,
      `{"jsonrpc":"2.0","id":${JSON.stringify(oldId)},"result":4}`
    );

    expect(harness.peer.closes).toEqual([
      { code: 1002, reason: 'unknown JSON-RPC response id' }
    ]);
    expect(runtime.responses).toHaveLength(2);
    expectNoActive(harness.broker);
  });

  it('turns a valid-id profile error into a tombstone before its exact write', () => {
    const harness = createHarness();
    const invalidParams =
      '{"jsonrpc":"2.0","id":"peer-a","method":"status.get","params":null}';

    harness.broker.handlePeerText(harness.generation, invalidParams);
    expect(harness.peer.writes).toEqual([
      '{"jsonrpc":"2.0","id":"peer-a","error":{"code":-32602,"message":"Invalid params"}}'
    ]);
    harness.broker.handlePeerText(harness.generation, invalidParams);
    expect(harness.peer.closes).toEqual([
      { code: 1002, reason: 'duplicate JSON-RPC request id' }
    ]);
    expectNoActive(harness.broker);
  });
});

interface HarnessOptions {
  readonly dispatchInbound?: (
    action: InboundDispatchAction
  ) => InboundDispatchResult | Promise<InboundDispatchResult>;
  readonly observeNotification?: (action: unknown) => void;
  readonly inboundTimeoutMs?: number;
  readonly outboundGlobalCapacity?: number;
  readonly outboundPerGenerationCapacity?: number;
  readonly inboundGlobalCapacity?: number;
  readonly inboundPerGenerationCapacity?: number;
  readonly inboundTombstoneCapacity?: number;
  readonly inboundTombstoneTtlMs?: number;
  readonly outboundTombstoneCapacity?: number;
  readonly writeText?: (frame: string) => void | Promise<void>;
  readonly profileLimits?: ProfileLimits;
}

function createHarness(options: HarnessOptions = {}) {
  const profileLimits =
    options.profileLimits ?? DEFAULT_JSON_RPC_20_TEXT_LIMITS;
  const profile = new JsonRpc20TextProfile(profileLimits);
  const peer = createPeer(options.writeText);
  const dispatches: InboundDispatchAction[] = [];
  const ownerToken = {};
  const broker = new WebSocketRequestBroker({
    profiles: [profile],
    profileLimits,
    outboundGlobalCapacity: options.outboundGlobalCapacity ?? 8,
    outboundPerGenerationCapacity:
      options.outboundPerGenerationCapacity ?? 8,
    inboundGlobalCapacity: options.inboundGlobalCapacity ?? 8,
    inboundPerGenerationCapacity:
      options.inboundPerGenerationCapacity ?? 8,
    outboundTombstoneCapacity: options.outboundTombstoneCapacity ?? 8,
    inboundTombstoneCapacity: options.inboundTombstoneCapacity ?? 8,
    outboundTombstoneTtlMs: 60_000,
    inboundTombstoneTtlMs: options.inboundTombstoneTtlMs ?? 60_000,
    inboundTimeoutMs: options.inboundTimeoutMs ?? 1_000,
    dispatchInbound(action) {
      dispatches.push(action);
      return options.dispatchInbound?.(action) ?? {
        kind: 'success',
        result: opaqueResult(profile, 'null')
      };
    },
    ...(options.observeNotification === undefined
      ? {}
      : { observeNotification: options.observeNotification })
  });
  const generation = broker.attachGeneration({
    connectionId: 'connection-a',
    socketGeneration: 'generation-a',
    serviceId: 'example/chat',
    websocketEntryId: 'entry-a',
    ownerToken,
    profile: 'jsonrpc-2.0-text',
    outboundIdPrefix: 'generation-a',
    writer: peer,
    acceptInboundMethod: (method) => method === 'status.get'
  });
  return {
    broker,
    dispatches,
    generation,
    ownerToken,
    peer,
    profile,
    request(
      runtime: TestRuntime,
      requestId: string,
      extra: { readonly deadlineAtMs?: number } = {}
    ) {
      broker.handleRuntimeRequest(generation, {
        source: runtime,
        requestId,
        serviceId: 'example/chat',
        websocketEntryId: 'entry-a',
        ownerToken,
        profile: 'jsonrpc-2.0-text',
        method: 'status.get',
        payloadBytes: Buffer.from('{"input":true}'),
        ...extra
      });
    }
  };
}

interface TestPeer {
  readonly writes: string[];
  readonly closes: Array<{ code: number; reason: string }>;
  writeText(frame: string): void | Promise<void>;
  close(code: number, reason: string): void;
}

function createPeer(
  override?: (frame: string) => void | Promise<void>
): TestPeer {
  const writes: string[] = [];
  const closes: Array<{ code: number; reason: string }> = [];
  return {
    writes,
    closes,
    writeText(frame) {
      if (override !== undefined) {
        return override(frame);
      }
      writes.push(frame);
    },
    close(code, reason) {
      closes.push({ code, reason });
    }
  };
}

interface TestRuntime extends BrokerRuntimeSource {
  readonly responses: Array<
    BrokerRuntimeResponse & { readonly payloadText?: string }
  >;
}

function createRuntime(sessionToken: string): TestRuntime {
  const responses: Array<
    BrokerRuntimeResponse & { readonly payloadText?: string }
  > = [];
  return {
    sender: {},
    sessionToken,
    responses,
    respond(response) {
      responses.push({
        ...response,
        ...(response.payloadBytes === undefined
          ? {}
          : {
              payloadText: Buffer.from(response.payloadBytes).toString('utf8')
            })
      });
    }
  };
}

function outboundId(frame: string): string {
  const id = (JSON.parse(frame) as { id: unknown }).id;
  if (typeof id !== 'string') {
    throw new Error('expected outbound string id');
  }
  return id;
}

function opaqueResult(
  profile: JsonRpc20TextProfile,
  source: string
): OpaquePayload {
  return profile.fromRuntimePayload(
    Buffer.from(source),
    'inboundResult',
    DEFAULT_JSON_RPC_20_TEXT_LIMITS
  );
}

function deferred<T>(): {
  readonly promise: Promise<T>;
  resolve(value: T): void;
} {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((settle) => {
    resolve = settle;
  });
  return { promise, resolve };
}

async function flush(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
}

function expectNoActive(broker: WebSocketRequestBroker): void {
  expect(broker.debugSnapshot()).toMatchObject({
    outboundPeerEntries: 0,
    outboundRuntimeEntries: 0,
    inboundActiveEntries: 0,
    outboundGenerationActive: 0,
    inboundGenerationActive: 0,
    timerCount: 0,
    terminalLeaseCount: 0
  });
}
