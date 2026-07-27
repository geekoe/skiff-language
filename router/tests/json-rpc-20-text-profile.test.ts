import { describe, expect, it } from 'vitest';

import {
  DEFAULT_JSON_RPC_20_TEXT_LIMITS,
  JsonRpc20TextProfile,
  type OpaquePeerId,
  type ProfileAction
} from '../src/protocol/jsonRpc20TextProfile.js';
import {
  DEFAULT_JSON_RPC_20_TEXT_LIMITS as CONTRACTS_DEFAULT_JSON_RPC_20_TEXT_LIMITS
} from '../src/protocol/jsonRpc20TextProfileContracts.js';
import {
  JsonRpc20TextProfile as JsonRpc20TextProfileImplementation
} from '../src/protocol/jsonRpc20TextProfileImplementation.js';
import {
  DEFAULT_JSON_RPC_20_TEXT_LIMITS as INDEX_DEFAULT_JSON_RPC_20_TEXT_LIMITS,
  JsonRpc20TextProfile as IndexJsonRpc20TextProfile
} from '../src/index.js';

describe('jsonrpc-2.0-text profile', () => {
  it('preserves class and default-limits identity across every public facade', () => {
    expect(JsonRpc20TextProfile).toBe(JsonRpc20TextProfileImplementation);
    expect(JsonRpc20TextProfile).toBe(IndexJsonRpc20TextProfile);
    expect(DEFAULT_JSON_RPC_20_TEXT_LIMITS).toBe(
      CONTRACTS_DEFAULT_JSON_RPC_20_TEXT_LIMITS
    );
    expect(DEFAULT_JSON_RPC_20_TEXT_LIMITS).toBe(
      INDEX_DEFAULT_JSON_RPC_20_TEXT_LIMITS
    );
  });

  it('classifies a strict request while preserving opaque params', () => {
    const profile = new JsonRpc20TextProfile();

    const action = profile.classifyText(
      '{"jsonrpc":"2.0","id":"peer-1","method":"chat.send","params":{"n":9007199254740993}}',
      DEFAULT_JSON_RPC_20_TEXT_LIMITS
    );

    expect(action).toMatchObject({
      kind: 'request',
      id: { kind: 'string', value: 'peer-1' },
      method: 'chat.send'
    });
    if (action.kind !== 'request') {
      throw new Error('expected request');
    }
    expect(profile.opaqueJsonText(action.params)).toBe(
      '{"n":9007199254740993}'
    );
  });

  it.each([
    {
      frame:
        '{"jsonrpc":"2.0","id":1e0,"method":"status.get","params":[]}',
      key: 'n:1',
      encodedId: '1'
    },
    {
      frame:
        '{"jsonrpc":"2.0","id":-0,"method":"status.get","params":[]}',
      key: 'n:0',
      encodedId: '0'
    },
    {
      frame:
        '{"jsonrpc":"2.0","id":"request-1","method":"status.get","params":[]}',
      key: 's:request-1',
      encodedId: '"request-1"'
    }
  ])('canonicalizes a legal typed id for $frame', ({ frame, key, encodedId }) => {
    const profile = new JsonRpc20TextProfile();
    const action = profile.classifyText(
      frame,
      DEFAULT_JSON_RPC_20_TEXT_LIMITS
    );
    expect(action.kind).toBe('request');
    if (action.kind !== 'request') {
      throw new Error('expected request');
    }
    expect(profile.peerIdKey(action.id)).toBe(key);
    expect(profile.encodeResult(
      action.id,
      profile.fromRuntimePayload(
        Buffer.from('null'),
        'inboundResult',
        DEFAULT_JSON_RPC_20_TEXT_LIMITS
      )
    )).toContain(`"id":${encodedId}`);
  });

  it('classifies ordinary notifications without creating a response id', () => {
    const profile = new JsonRpc20TextProfile();
    const action = classify(
      profile,
      '{"jsonrpc":"2.0","method":"telemetry.observe","params":{"ok":true}}'
    );

    expect(action).toMatchObject({
      kind: 'ignoredNotification',
      method: 'telemetry.observe'
    });
    if (action.kind !== 'ignoredNotification' || action.params === undefined) {
      throw new Error('expected notification params');
    }
    expect(profile.opaqueJsonText(action.params)).toBe('{"ok":true}');
  });

  it('accepts only the exact cancel notification spelling', () => {
    const profile = new JsonRpc20TextProfile();

    expect(classify(
      profile,
      '{"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":4}}'
    )).toEqual({
      kind: 'cancel',
      id: { kind: 'safeInteger', value: 4 }
    });
    expect(classify(
      profile,
      '{"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":4,"reason":"x"}}'
    )).toEqual({
      kind: 'platformError',
      id: null,
      error: { kind: 'invalidRequest' }
    });
  });

  it('keeps success result and remote error data as lossless opaque JSON', () => {
    const profile = new JsonRpc20TextProfile();
    const success = classify(
      profile,
      '{"jsonrpc":"2.0","id":"out:1","result":{"large":9007199254740993}}'
    );
    expect(success.kind).toBe('response');
    if (
      success.kind !== 'response' ||
      success.terminal.kind !== 'success'
    ) {
      throw new Error('expected success response');
    }
    expect(Buffer.from(
      profile.toRuntimePayload(
        success.terminal.result,
        DEFAULT_JSON_RPC_20_TEXT_LIMITS
      )
    ).toString('utf8')).toBe('{"large":9007199254740993}');

    const failure = classify(
      profile,
      '{"jsonrpc":"2.0","id":"out:2","error":{"code":-32009,"message":"peer failed","data":{"large":9007199254740993}}}'
    );
    expect(failure.kind).toBe('response');
    if (
      failure.kind !== 'response' ||
      failure.terminal.kind !== 'remoteError'
    ) {
      throw new Error('expected remote error response');
    }
    expect(failure.terminal).toMatchObject({
      code: -32009,
      message: 'peer failed',
      dataPresent: true
    });
    expect(profile.opaqueJsonText(failure.terminal.data!)).toBe(
      '{"large":9007199254740993}'
    );
  });

  it.each([
    ['malformed JSON', '{', 'parse'],
    ['batch', '[]', 'invalidRequest'],
    ['scalar', '1', 'invalidRequest'],
    [
      'unknown request member',
      '{"jsonrpc":"2.0","id":"a","method":"m","params":{},"extra":true}',
      'invalidRequest'
    ],
    [
      'duplicate control member',
      '{"jsonrpc":"2.0","id":"a","id":"b","method":"m","params":{}}',
      'invalidRequest'
    ]
  ])('maps %s to a fixed platform error', (_name, frame, kind) => {
    expect(classify(new JsonRpc20TextProfile(), frame)).toEqual({
      kind: 'platformError',
      id: null,
      error: { kind }
    });
  });

  it.each([
    '{"jsonrpc":"2.0","id":null,"method":"m","params":{}}',
    '{"jsonrpc":"2.0","id":"","method":"m","params":{}}',
    '{"jsonrpc":"2.0","id":1.5,"method":"m","params":{}}',
    '{"jsonrpc":"2.0","id":1e-324,"method":"m","params":{}}',
    '{"jsonrpc":"2.0","id":1.0000000000000000001,"method":"m","params":{}}',
    '{"jsonrpc":"2.0","id":9007199254740992,"method":"m","params":{}}'
  ])('rejects an illegal request id without echoing it: %s', (frame) => {
    expect(classify(new JsonRpc20TextProfile(), frame)).toEqual({
      kind: 'platformError',
      id: null,
      error: { kind: 'invalidRequest' }
    });
  });

  it.each([
    '{"jsonrpc":"2.0","id":"a","method":"m"}',
    '{"jsonrpc":"2.0","id":"a","method":"m","params":null}',
    '{"jsonrpc":"2.0","id":"a","method":"m","params":"bad"}'
  ])('echoes a valid id for invalid params: %s', (frame) => {
    expect(classify(new JsonRpc20TextProfile(), frame)).toEqual({
      kind: 'platformError',
      id: { kind: 'string', value: 'a' },
      error: { kind: 'invalidParams' }
    });
  });

  it.each([
    '{"jsonrpc":"2.0","id":1,"result":null}',
    '{"jsonrpc":"2.0","id":"a","result":null,"extra":true}',
    '{"jsonrpc":"2.0","id":"a","result":null,"error":{"code":1,"message":"x"}}',
    '{"jsonrpc":"2.0","id":"a","error":{"code":1.5,"message":"x"}}',
    '{"jsonrpc":"2.0","id":"a","error":{"code":1e-324,"message":"x"}}',
    '{"jsonrpc":"2.0","id":"a","error":{"code":1,"message":""}}',
    '{"jsonrpc":"2.0","id":"a","error":{"code":1,"message":"x","extra":true}}'
  ])('closes on a malformed response: %s', (frame) => {
    expect(classify(new JsonRpc20TextProfile(), frame)).toEqual({
      kind: 'close',
      code: 1002,
      reason: 'invalid JSON-RPC response'
    });
  });

  it('preserves null result and remote data presence independently', () => {
    const profile = new JsonRpc20TextProfile();
    const result = classify(
      profile,
      '{"jsonrpc":"2.0","id":"a","result":null}'
    );
    if (result.kind !== 'response' || result.terminal.kind !== 'success') {
      throw new Error('expected success');
    }
    expect(profile.opaqueJsonText(result.terminal.result)).toBe('null');

    const noData = classify(
      profile,
      '{"jsonrpc":"2.0","id":"b","error":{"code":1,"message":"x"}}'
    );
    const nullData = classify(
      profile,
      '{"jsonrpc":"2.0","id":"c","error":{"code":1,"message":"x","data":null}}'
    );
    expect(noData).toMatchObject({
      terminal: { kind: 'remoteError', dataPresent: false }
    });
    expect(nullData).toMatchObject({
      terminal: { kind: 'remoteError', dataPresent: true }
    });
  });

  it('allows duplicate business members while keeping control members strict', () => {
    const profile = new JsonRpc20TextProfile();
    const action = classify(
      profile,
      '{"jsonrpc":"2.0","id":"a","method":"m","params":{"business":1,"business":2}}'
    );
    expect(action.kind).toBe('request');
    if (action.kind !== 'request') {
      throw new Error('expected request');
    }
    expect(profile.opaqueJsonText(action.params)).toBe(
      '{"business":1,"business":2}'
    );
  });

  it('enforces frame, depth, node, and string limits as close 1009', () => {
    const profile = new JsonRpc20TextProfile();
    for (const [frame, limits] of [
      [
        '{"jsonrpc":"2.0","id":"a","method":"m","params":{}}',
        { ...DEFAULT_JSON_RPC_20_TEXT_LIMITS, maxTextBytes: 10 }
      ],
      [
        '{"jsonrpc":"2.0","id":"a","method":"m","params":{"x":{"y":1}}}',
        { ...DEFAULT_JSON_RPC_20_TEXT_LIMITS, maxJsonDepth: 2 }
      ],
      [
        '{"jsonrpc":"2.0","id":"a","method":"m","params":[1,2,3]}',
        { ...DEFAULT_JSON_RPC_20_TEXT_LIMITS, maxJsonNodes: 4 }
      ],
      [
        '{"jsonrpc":"2.0","id":"a","method":"long","params":{}}',
        { ...DEFAULT_JSON_RPC_20_TEXT_LIMITS, maxStringBytes: 3 }
      ]
    ] as const) {
      expect(profile.classifyText(frame, limits)).toEqual({
        kind: 'close',
        code: 1009,
        reason: 'JSON-RPC text frame exceeds profile limits'
      });
    }
  });

  it('rejects a typed id before dispatch when the smallest terminal cannot fit', () => {
    const limits = {
      ...DEFAULT_JSON_RPC_20_TEXT_LIMITS,
      maxTextBytes: 85
    };
    const profile = new JsonRpc20TextProfile(limits);

    expect(profile.classifyText(
      `{"jsonrpc":"2.0","id":"${'x'.repeat(35)}","method":"m","params":{}}`,
      limits
    )).toEqual({
      kind: 'close',
      code: 1009,
      reason: 'JSON-RPC text frame exceeds profile limits'
    });
  });

  it('validates runtime payload purposes without round-tripping numbers', () => {
    const profile = new JsonRpc20TextProfile();
    const params = profile.fromRuntimePayload(
      Buffer.from('{"large":9007199254740993}'),
      'outboundParams',
      DEFAULT_JSON_RPC_20_TEXT_LIMITS
    );
    expect(profile.opaqueJsonText(params)).toBe(
      '{"large":9007199254740993}'
    );
    expect(() => profile.fromRuntimePayload(
      Buffer.from('null'),
      'outboundParams',
      DEFAULT_JSON_RPC_20_TEXT_LIMITS
    )).toThrow(/object or array/);
    expect(profile.opaqueJsonText(profile.fromRuntimePayload(
      Buffer.from('null'),
      'inboundResult',
      DEFAULT_JSON_RPC_20_TEXT_LIMITS
    ))).toBe('null');
  });

  it('encodes request, cancel, result and fixed platform errors exactly', () => {
    const profile = new JsonRpc20TextProfile();
    const id = { kind: 'string', value: 'g:1' } satisfies OpaquePeerId;
    const params = profile.fromRuntimePayload(
      Buffer.from('{"large":9007199254740993}'),
      'outboundParams',
      DEFAULT_JSON_RPC_20_TEXT_LIMITS
    );
    expect(profile.encodeOutboundRequest({
      id,
      method: 'status.get',
      params
    })).toBe(
      '{"jsonrpc":"2.0","id":"g:1","method":"status.get","params":{"large":9007199254740993}}'
    );
    expect(profile.encodeCancel(id)).toBe(
      '{"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":"g:1"}}'
    );
    expect(profile.encodePlatformError(
      id,
      { kind: 'serverBusy' }
    )).toBe(
      '{"jsonrpc":"2.0","id":"g:1","error":{"code":-32000,"message":"Server busy"}}'
    );
  });
});

function classify(
  profile: JsonRpc20TextProfile,
  frame: string
): ProfileAction {
  return profile.classifyText(frame, DEFAULT_JSON_RPC_20_TEXT_LIMITS);
}
