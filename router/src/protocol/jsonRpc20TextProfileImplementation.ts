import { TextDecoder, TextEncoder } from 'node:util';

import {
  LosslessJsonLimitError,
  type LosslessJsonNode,
  type LosslessJsonObjectNode,
  losslessJsonSlice,
  parseLosslessJson,
  uniqueObjectMembers
} from './losslessJson.js';
import {
  DEFAULT_JSON_RPC_20_TEXT_LIMITS,
  type OpaquePayload,
  type OpaquePeerId,
  type OutboundIdGeneration,
  type PlatformRpcError,
  type ProfileAction,
  type ProfileLimits,
  type WebSocketRpcProfileAdapter
} from './jsonRpc20TextProfileContracts.js';

const UTF8_DECODER = new TextDecoder('utf-8', {
  fatal: true,
  ignoreBOM: true
});
const UTF8_ENCODER = new TextEncoder();
const opaquePayloadText = new WeakMap<object, string>();

export class JsonRpc20TextProfile implements WebSocketRpcProfileAdapter {
  readonly profile = 'jsonrpc-2.0-text' as const;

  constructor(
    private readonly encodingLimits: ProfileLimits =
      DEFAULT_JSON_RPC_20_TEXT_LIMITS
  ) {
    validateLimits(encodingLimits);
  }

  classifyText(frame: string, limits: ProfileLimits): ProfileAction {
    validateLimits(limits);
    if (
      !encodedFrameFitsLimits(
        platformErrorFrame(null, { kind: 'cancelled' }),
        limits
      ) ||
      Buffer.byteLength(frame, 'utf8') > limits.maxTextBytes
    ) {
      return profileLimitClose();
    }

    let root: LosslessJsonNode;
    try {
      root = parseLosslessJson(frame, limits);
    } catch (error) {
      if (error instanceof LosslessJsonLimitError) {
        return profileLimitClose();
      }
      return {
        kind: 'platformError',
        id: null,
        error: { kind: 'parse' }
      };
    }

    if (root.kind === 'array' || root.kind !== 'object') {
      return invalidRequest();
    }

    const fieldNames = new Set(root.members.map(({ key }) => key));
    const responseCandidate =
      fieldNames.has('result') ||
      fieldNames.has('error') ||
      (fieldNames.has('id') && !fieldNames.has('method'));
    if (responseCandidate) {
      return this.classifyResponse(frame, root);
    }
    if (
      fieldNames.has('method') &&
      !fieldNames.has('result') &&
      !fieldNames.has('error')
    ) {
      return this.classifyRequestOrNotification(frame, root, limits);
    }
    return invalidRequest();
  }

  peerIdKey(id: OpaquePeerId): string {
    return id.kind === 'string'
      ? `s:${id.value}`
      : `n:${canonicalSafeInteger(id.value)}`;
  }

  nextOutboundId(generation: OutboundIdGeneration): OpaquePeerId {
    if (generation.randomPrefix.length === 0) {
      throw new Error('outbound id prefix must be non-empty');
    }
    const sequence = generation.takeSequence();
    if (sequence < 0n) {
      throw new Error('outbound id sequence must not be negative');
    }
    return {
      kind: 'string',
      value: `${generation.randomPrefix}:${sequence}`
    };
  }

  fromRuntimePayload(
    bytes: Uint8Array,
    purpose: 'outboundParams' | 'inboundResult',
    limits: ProfileLimits
  ): OpaquePayload {
    validateLimits(limits);
    if (bytes.byteLength === 0) {
      throw new Error('runtime JSON payload must be present');
    }
    if (bytes.byteLength > limits.maxTextBytes) {
      throw new Error('runtime JSON payload exceeds the profile text limit');
    }
    let source: string;
    try {
      source = UTF8_DECODER.decode(bytes);
    } catch {
      throw new Error('runtime JSON payload must be valid UTF-8');
    }
    let root: LosslessJsonNode;
    try {
      root = parseLosslessJson(source, limits);
    } catch (error) {
      throw new Error(
        error instanceof LosslessJsonLimitError
          ? 'runtime JSON payload exceeds profile limits'
          : 'runtime JSON payload must be valid JSON'
      );
    }
    if (
      purpose === 'outboundParams' &&
      root.kind !== 'object' &&
      root.kind !== 'array'
    ) {
      throw new Error('outbound params must be a JSON object or array');
    }
    return createOpaquePayload(losslessJsonSlice(source, root));
  }

  toRuntimePayload(
    payload: OpaquePayload,
    limits: ProfileLimits
  ): Uint8Array {
    validateLimits(limits);
    const source = this.opaqueJsonText(payload);
    if (Buffer.byteLength(source, 'utf8') > limits.maxTextBytes) {
      throw new Error('opaque JSON payload exceeds the profile text limit');
    }
    try {
      parseLosslessJson(source, limits);
    } catch (error) {
      throw new Error(
        error instanceof LosslessJsonLimitError
          ? 'opaque JSON payload exceeds profile limits'
          : 'opaque JSON payload is invalid'
      );
    }
    return UTF8_ENCODER.encode(source);
  }

  opaqueJsonText(payload: OpaquePayload): string {
    const text = opaquePayloadText.get(payload as object);
    if (text === undefined) {
      throw new Error('opaque JSON payload was not created by this profile');
    }
    return text;
  }

  encodeOutboundRequest(input: {
    readonly id: OpaquePeerId;
    readonly method: string;
    readonly params: OpaquePayload;
  }): string {
    if (input.method.length === 0) {
      throw new Error('outbound JSON-RPC method must be non-empty');
    }
    const frame =
      `{"jsonrpc":"2.0","id":${encodePeerId(input.id)}` +
      `,"method":${JSON.stringify(input.method)}` +
      `,"params":${this.opaqueJsonText(input.params)}}`;
    return this.assertEncodedFrame(frame);
  }

  encodeCancel(id: OpaquePeerId): string {
    return this.assertEncodedFrame(
      '{"jsonrpc":"2.0","method":"$/cancelRequest","params":' +
        `{"id":${encodePeerId(id)}}}`
    );
  }

  encodeResult(id: OpaquePeerId, result: OpaquePayload): string {
    return this.assertEncodedFrame(
      `{"jsonrpc":"2.0","id":${encodePeerId(id)}` +
        `,"result":${this.opaqueJsonText(result)}}`
    );
  }

  encodePlatformError(
    id: OpaquePeerId | null,
    error: PlatformRpcError
  ): string {
    return this.assertEncodedFrame(platformErrorFrame(id, error));
  }

  private classifyRequestOrNotification(
    source: string,
    root: LosslessJsonObjectNode,
    limits: ProfileLimits
  ): ProfileAction {
    const members = uniqueObjectMembers(root);
    if (members === undefined) {
      return invalidRequest();
    }
    const hasId = members.has('id');
    const allowed = hasId
      ? new Set(['jsonrpc', 'id', 'method', 'params'])
      : new Set(['jsonrpc', 'method', 'params']);
    if (
      !hasOnlyFields(members, allowed) ||
      !members.has('jsonrpc') ||
      !members.has('method') ||
      !isExactJsonRpcVersion(members.get('jsonrpc'))
    ) {
      return invalidRequest();
    }

    const methodNode = members.get('method');
    if (
      methodNode?.kind !== 'string' ||
      methodNode.value.length === 0
    ) {
      return invalidRequest();
    }

    if (!hasId) {
      if (methodNode.value === '$/cancelRequest') {
        const cancel = classifyCancel(members.get('params'));
        if (
          cancel.kind === 'cancel' &&
          !encodedFrameFitsLimits(
            platformErrorFrame(cancel.id, { kind: 'cancelled' }),
            limits
          )
        ) {
          return profileLimitClose();
        }
        return cancel;
      }
      const paramsNode = members.get('params');
      if (
        paramsNode !== undefined &&
        paramsNode.kind !== 'object' &&
        paramsNode.kind !== 'array'
      ) {
        return invalidRequest();
      }
      return {
        kind: 'ignoredNotification',
        method: methodNode.value,
        ...(paramsNode === undefined
          ? {}
          : { params: createOpaquePayload(losslessJsonSlice(source, paramsNode)) })
      };
    }

    const id = parsePeerId(members.get('id'));
    if (id === undefined) {
      return invalidRequest();
    }
    if (
      !encodedFrameFitsLimits(
        platformErrorFrame(id, { kind: 'cancelled' }),
        limits
      )
    ) {
      return profileLimitClose();
    }
    const paramsNode = members.get('params');
    if (
      paramsNode === undefined ||
      (paramsNode.kind !== 'object' && paramsNode.kind !== 'array')
    ) {
      return {
        kind: 'platformError',
        id,
        error: { kind: 'invalidParams' }
      };
    }
    return {
      kind: 'request',
      id,
      method: methodNode.value,
      params: createOpaquePayload(losslessJsonSlice(source, paramsNode))
    };
  }

  private classifyResponse(
    source: string,
    root: LosslessJsonObjectNode
  ): ProfileAction {
    const invalid = invalidResponse();
    const members = uniqueObjectMembers(root);
    if (
      members === undefined ||
      !isExactJsonRpcVersion(members.get('jsonrpc'))
    ) {
      return invalid;
    }
    const resultNode = members.get('result');
    const errorNode = members.get('error');
    if ((resultNode === undefined) === (errorNode === undefined)) {
      return invalid;
    }
    const idNode = members.get('id');
    if (idNode?.kind !== 'string' || idNode.value.length === 0) {
      return invalid;
    }
    const id = { kind: 'string', value: idNode.value } as const;

    if (resultNode !== undefined) {
      if (
        !hasExactFields(members, new Set(['jsonrpc', 'id', 'result']))
      ) {
        return invalid;
      }
      return {
        kind: 'response',
        id,
        terminal: {
          kind: 'success',
          result: createOpaquePayload(losslessJsonSlice(source, resultNode))
        }
      };
    }

    if (
      !hasExactFields(members, new Set(['jsonrpc', 'id', 'error'])) ||
      errorNode?.kind !== 'object'
    ) {
      return invalid;
    }
    const errorMembers = uniqueObjectMembers(errorNode);
    if (
      errorMembers === undefined ||
      !hasExactFields(
        errorMembers,
        errorMembers.has('data')
          ? new Set(['code', 'message', 'data'])
          : new Set(['code', 'message'])
      )
    ) {
      return invalid;
    }
    const code = parseSafeInteger(errorMembers.get('code'));
    const message = errorMembers.get('message');
    if (
      code === undefined ||
      message?.kind !== 'string' ||
      message.value.length === 0
    ) {
      return invalid;
    }
    const dataNode = errorMembers.get('data');
    return {
      kind: 'response',
      id,
      terminal: {
        kind: 'remoteError',
        code,
        message: message.value,
        dataPresent: dataNode !== undefined,
        ...(dataNode === undefined
          ? {}
          : { data: createOpaquePayload(losslessJsonSlice(source, dataNode)) })
      }
    };
  }

  private assertEncodedFrame(frame: string): string {
    if (Buffer.byteLength(frame, 'utf8') > this.encodingLimits.maxTextBytes) {
      throw new Error('encoded JSON-RPC frame exceeds the profile text limit');
    }
    try {
      parseLosslessJson(frame, this.encodingLimits);
    } catch (error) {
      throw new Error(
        error instanceof LosslessJsonLimitError
          ? 'encoded JSON-RPC frame exceeds profile limits'
          : 'encoded JSON-RPC frame is invalid'
      );
    }
    return frame;
  }
}

function classifyCancel(params: LosslessJsonNode | undefined): ProfileAction {
  if (params?.kind !== 'object') {
    return invalidRequest();
  }
  const members = uniqueObjectMembers(params);
  if (
    members === undefined ||
    !hasExactFields(members, new Set(['id']))
  ) {
    return invalidRequest();
  }
  const id = parsePeerId(members.get('id'));
  return id === undefined ? invalidRequest() : { kind: 'cancel', id };
}

function parsePeerId(
  node: LosslessJsonNode | undefined
): OpaquePeerId | undefined {
  if (node?.kind === 'string') {
    return node.value.length === 0
      ? undefined
      : { kind: 'string', value: node.value };
  }
  const value = parseSafeInteger(node);
  return value === undefined
    ? undefined
    : { kind: 'safeInteger', value: canonicalSafeInteger(value) };
}

function parseSafeInteger(
  node: LosslessJsonNode | undefined
): number | undefined {
  if (node?.kind !== 'number') {
    return undefined;
  }
  const match =
    /^(-?)([0-9]+)(?:\.([0-9]+))?(?:[eE]([+-]?[0-9]+))?$/.exec(
      node.lexeme
    );
  if (match === null) {
    return undefined;
  }
  const negative = match[1] === '-';
  const integerDigits = match[2]!;
  const fractionDigits = match[3] ?? '';
  const coefficient = `${integerDigits}${fractionDigits}`;
  if (/^0+$/.test(coefficient)) {
    return 0;
  }

  const exponentSpelling = match[4] ?? '0';
  const normalizedExponent = exponentSpelling
    .replace(/^[+-]/, '')
    .replace(/^0+/, '');
  if (normalizedExponent.length > 15) {
    return undefined;
  }
  const exponent = BigInt(exponentSpelling);
  const scale = exponent - BigInt(fractionDigits.length);
  let exactDigits: string;
  if (scale >= 0n) {
    const significant = coefficient.replace(/^0+/, '');
    if (BigInt(significant.length) + scale > 16n) {
      return undefined;
    }
    exactDigits = `${significant}${'0'.repeat(Number(scale))}`;
  } else {
    const removedDigits = -scale;
    if (removedDigits > BigInt(coefficient.length)) {
      return undefined;
    }
    const removedCount = Number(removedDigits);
    const removed = coefficient.slice(coefficient.length - removedCount);
    if (!/^0*$/.test(removed)) {
      return undefined;
    }
    exactDigits = coefficient
      .slice(0, coefficient.length - removedCount)
      .replace(/^0+/, '');
    if (exactDigits.length === 0) {
      return 0;
    }
  }

  let exact = BigInt(exactDigits);
  if (negative) {
    exact = -exact;
  }
  if (
    exact > MAX_SAFE_INTEGER_BIGINT ||
    exact < -MAX_SAFE_INTEGER_BIGINT
  ) {
    return undefined;
  }
  return Number(exact);
}

function canonicalSafeInteger(value: number): number {
  if (!Number.isSafeInteger(value)) {
    throw new Error('peer id must be a JavaScript safe integer');
  }
  return Object.is(value, -0) ? 0 : value;
}

function isExactJsonRpcVersion(node: LosslessJsonNode | undefined): boolean {
  return node?.kind === 'string' && node.value === '2.0';
}

function hasExactFields(
  members: ReadonlyMap<string, LosslessJsonNode>,
  expected: ReadonlySet<string>
): boolean {
  if (members.size !== expected.size) {
    return false;
  }
  for (const field of expected) {
    if (!members.has(field)) {
      return false;
    }
  }
  return true;
}

function hasOnlyFields(
  members: ReadonlyMap<string, LosslessJsonNode>,
  allowed: ReadonlySet<string>
): boolean {
  for (const field of members.keys()) {
    if (!allowed.has(field)) {
      return false;
    }
  }
  return true;
}

function encodePeerId(id: OpaquePeerId): string {
  return id.kind === 'string'
    ? JSON.stringify(id.value)
    : JSON.stringify(canonicalSafeInteger(id.value));
}

function createOpaquePayload(source: string): OpaquePayload {
  const payload = Object.freeze({}) as OpaquePayload;
  opaquePayloadText.set(payload as object, source);
  return payload;
}

function invalidRequest(): ProfileAction {
  return {
    kind: 'platformError',
    id: null,
    error: { kind: 'invalidRequest' }
  };
}

function invalidResponse(): ProfileAction {
  return {
    kind: 'close',
    code: 1002,
    reason: 'invalid JSON-RPC response'
  };
}

function profileLimitClose(): ProfileAction {
  return {
    kind: 'close',
    code: 1009,
    reason: 'JSON-RPC text frame exceeds profile limits'
  };
}

function platformErrorFrame(
  id: OpaquePeerId | null,
  error: PlatformRpcError
): string {
  const fixed = PLATFORM_ERRORS[error.kind];
  return (
    `{"jsonrpc":"2.0","id":${id === null ? 'null' : encodePeerId(id)}` +
    `,"error":{"code":${fixed.code},"message":${JSON.stringify(fixed.message)}}}`
  );
}

function encodedFrameFitsLimits(
  frame: string,
  limits: ProfileLimits
): boolean {
  if (Buffer.byteLength(frame, 'utf8') > limits.maxTextBytes) {
    return false;
  }
  try {
    parseLosslessJson(frame, limits);
    return true;
  } catch {
    return false;
  }
}

function validateLimits(limits: ProfileLimits): void {
  for (const value of [
    limits.maxTextBytes,
    limits.maxJsonDepth,
    limits.maxJsonNodes,
    limits.maxStringBytes
  ]) {
    if (!Number.isSafeInteger(value) || value <= 0) {
      throw new Error('profile limits must be positive safe integers');
    }
  }
}

const PLATFORM_ERRORS: Readonly<
  Record<PlatformRpcError['kind'], { readonly code: number; readonly message: string }>
> = Object.freeze({
  parse: { code: -32700, message: 'Parse error' },
  invalidRequest: { code: -32600, message: 'Invalid Request' },
  methodNotFound: { code: -32601, message: 'Method not found' },
  invalidParams: { code: -32602, message: 'Invalid params' },
  internal: { code: -32603, message: 'Internal error' },
  serverBusy: { code: -32000, message: 'Server busy' },
  timeout: { code: -32001, message: 'Request timed out' },
  cancelled: { code: -32800, message: 'Request cancelled' }
});

const MAX_SAFE_INTEGER_BIGINT = BigInt(Number.MAX_SAFE_INTEGER);
