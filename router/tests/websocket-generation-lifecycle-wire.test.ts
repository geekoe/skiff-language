import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

import {
  decodeRawWebSocketGenerationLifecycleControl,
  decodeWebSocketGenerationLifecycleControl,
  decodeWebSocketGenerationLifecycleFrame,
  encodeWebSocketGenerationLifecycleFrame,
  type WebSocketGenerationLifecycleControl,
  type WebSocketGenerationLifecycleDirection,
  type WebSocketGenerationLifecycleRequest,
  type WebSocketGenerationLifecycleResponse,
  assertWebSocketGenerationLifecycleResponseMatches
} from '../src/protocol/webSocketGenerationLifecycle.js';

interface ValidControl {
  name: string;
  direction: WebSocketGenerationLifecycleDirection;
  control: Record<string, unknown>;
}

interface ControlMutation {
  name: string;
  baseIndex: number;
  direction: WebSocketGenerationLifecycleDirection;
  setPath?: string;
  removePath?: string;
  value?: unknown;
}

interface RawInvalidControl {
  name: string;
  direction: WebSocketGenerationLifecycleDirection;
  rawJson: string;
}

interface ResponseCorrelation {
  name: string;
  requestIndex: number;
  responseIndex: number;
  matches: boolean;
  setPath?: string;
  value?: unknown;
}

interface LifecycleCorpus {
  validControls: ValidControl[];
  controlMutations: ControlMutation[];
  rawInvalidControls: RawInvalidControl[];
  responseCorrelations: ResponseCorrelation[];
}

const corpus = JSON.parse(
  readFileSync(
    new URL(
      '../../cross-system-fixtures/package-service-ecosystem/websocket-generation-lifecycle-wire.json',
      import.meta.url
    ),
    'utf8'
  )
) as LifecycleCorpus;

describe('WebSocket generation lifecycle shared wire', () => {
  it('round-trips exact acquire, release, ack, and typed rejection controls', () => {
    expect(corpus.validControls).toHaveLength(7);
    for (const fixture of corpus.validControls) {
      const decoded = decodeWebSocketGenerationLifecycleControl(
        fixture.control,
        fixture.direction
      );
      expect(decoded, fixture.name).toEqual(fixture.control);

      const decodedRaw = decodeRawWebSocketGenerationLifecycleControl(
        JSON.stringify(fixture.control),
        fixture.direction
      );
      expect(decodedRaw, `${fixture.name} raw`).toEqual(decoded);

      const frame = encodeWebSocketGenerationLifecycleFrame(decoded, fixture.direction);
      expect(
        decodeWebSocketGenerationLifecycleFrame(frame, fixture.direction),
        `${fixture.name} binary`
      ).toEqual(decoded);
    }
  });

  it('treats an exact duplicate release as the same idempotency key', () => {
    const original = decodeFixture(3);
    const duplicate = decodeFixture(6);
    expect(duplicate).toEqual(original);
  });

  it('rejects unknown, missing, extra, identity, tuple, request-id, and sender mutations', () => {
    expect(corpus.controlMutations).toHaveLength(24);
    for (const mutation of corpus.controlMutations) {
      const value = structuredClone(corpus.validControls[mutation.baseIndex]!.control);
      applyMutation(value, mutation);
      expect(
        () => decodeWebSocketGenerationLifecycleControl(value, mutation.direction),
        mutation.name
      ).toThrow();
      expect(
        () =>
          decodeRawWebSocketGenerationLifecycleControl(
            JSON.stringify(value),
            mutation.direction
          ),
        `${mutation.name} raw`
      ).toThrow();
    }
  });

  it('rejects duplicate JSON keys and non-empty binary payloads', () => {
    expect(corpus.rawInvalidControls).toHaveLength(2);
    for (const invalid of corpus.rawInvalidControls) {
      expect(
        () =>
          decodeRawWebSocketGenerationLifecycleControl(
            invalid.rawJson,
            invalid.direction
          ),
        invalid.name
      ).toThrow(/duplicate JSON object key/);
      expect(
        () =>
          decodeWebSocketGenerationLifecycleFrame(
            frameWithRawHeader(invalid.rawJson),
            invalid.direction
          ),
        `${invalid.name} binary`
      ).toThrow(/duplicate JSON object key/);
    }

    const acquire = decodeFixture(0);
    const validFrame = encodeWebSocketGenerationLifecycleFrame(
      acquire,
      'runtimeToRouter'
    );
    const withPayload = Buffer.concat([
      validFrame.subarray(0, 10),
      Buffer.from([0, 0, 0, 1]),
      validFrame.subarray(14),
      Buffer.from([1])
    ]);
    expect(() =>
      decodeWebSocketGenerationLifecycleFrame(withPayload, 'runtimeToRouter')
    ).toThrow(/payload must be empty/);
  });

  it('requires responses to echo the exact operation, request id, and tuple', () => {
    expect(corpus.responseCorrelations).toHaveLength(7);
    for (const correlation of corpus.responseCorrelations) {
      const requestValue = structuredClone(
        corpus.validControls[correlation.requestIndex]!.control
      );
      const responseValue = structuredClone(
        corpus.validControls[correlation.responseIndex]!.control
      );
      if (correlation.setPath !== undefined) {
        setPath(responseValue, correlation.setPath, correlation.value);
      }
      const requestControl = decodeWebSocketGenerationLifecycleControl(
        requestValue,
        corpus.validControls[correlation.requestIndex]!.direction
      );
      const responseControl = decodeWebSocketGenerationLifecycleControl(
        responseValue,
        corpus.validControls[correlation.responseIndex]!.direction
      );
      const request = asRequest(requestControl);
      const response = asResponse(responseControl);
      if (correlation.matches) {
        expect(
          () => assertWebSocketGenerationLifecycleResponseMatches(request, response),
          correlation.name
        ).not.toThrow();
      } else {
        expect(
          () => assertWebSocketGenerationLifecycleResponseMatches(request, response),
          correlation.name
        ).toThrow();
      }
    }
  });
});

function decodeFixture(index: number): WebSocketGenerationLifecycleControl {
  const fixture = corpus.validControls[index]!;
  return decodeWebSocketGenerationLifecycleControl(fixture.control, fixture.direction);
}

function asRequest(
  control: WebSocketGenerationLifecycleControl
): WebSocketGenerationLifecycleRequest {
  if (control.action !== 'acquire' && control.action !== 'release') {
    throw new Error(`expected request, received ${control.action}`);
  }
  return control;
}

function asResponse(
  control: WebSocketGenerationLifecycleControl
): WebSocketGenerationLifecycleResponse {
  if (control.action !== 'ack' && control.action !== 'reject') {
    throw new Error(`expected response, received ${control.action}`);
  }
  return control;
}

function applyMutation(
  value: Record<string, unknown>,
  mutation: ControlMutation
): void {
  if (mutation.setPath !== undefined) {
    setPath(value, mutation.setPath, mutation.value);
    return;
  }
  if (mutation.removePath !== undefined) {
    removePath(value, mutation.removePath);
  }
}

function setPath(
  root: Record<string, unknown>,
  path: string,
  value: unknown
): void {
  const { owner, leaf } = pathOwner(root, path);
  owner[leaf] = value;
}

function removePath(root: Record<string, unknown>, path: string): void {
  const { owner, leaf } = pathOwner(root, path);
  delete owner[leaf];
}

function pathOwner(
  root: Record<string, unknown>,
  path: string
): { owner: Record<string, unknown>; leaf: string } {
  const segments = path.split('.');
  const leaf = segments.pop();
  if (leaf === undefined) throw new Error('mutation path must not be empty');
  let owner = root;
  for (const segment of segments) {
    const next = owner[segment];
    if (typeof next !== 'object' || next === null || Array.isArray(next)) {
      throw new Error(`mutation owner ${segment} must be an object`);
    }
    owner = next as Record<string, unknown>;
  }
  return { owner, leaf };
}

function frameWithRawHeader(rawJson: string): Buffer {
  const header = Buffer.from(rawJson, 'utf8');
  const frame = Buffer.alloc(14 + header.byteLength);
  frame.write('SKBF', 0, 'ascii');
  frame.writeUInt8(1, 4);
  frame.writeUInt8(1, 5);
  frame.writeUInt32BE(header.byteLength, 6);
  frame.writeUInt32BE(0, 10);
  header.copy(frame, 14);
  return frame;
}
