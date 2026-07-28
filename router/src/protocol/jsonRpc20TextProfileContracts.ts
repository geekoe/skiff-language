export type ProfileId = 'jsonrpc-2.0-text';

export type OpaquePeerId =
  | { readonly kind: 'string'; readonly value: string }
  | { readonly kind: 'safeInteger'; readonly value: number };

declare const opaquePayloadBrand: unique symbol;

export interface OpaquePayload {
  readonly [opaquePayloadBrand]: true;
}

export interface ProfileLimits {
  readonly maxTextBytes: number;
  readonly maxJsonDepth: number;
  readonly maxJsonNodes: number;
  readonly maxStringBytes: number;
}

export const DEFAULT_JSON_RPC_20_TEXT_LIMITS: ProfileLimits = Object.freeze({
  maxTextBytes: 1024 * 1024,
  maxJsonDepth: 64,
  maxJsonNodes: 100_000,
  maxStringBytes: 64 * 1024
});

export interface OutboundIdGeneration {
  readonly randomPrefix: string;
  takeSequence(): bigint;
}

export type ProfileResponse =
  | { readonly kind: 'success'; readonly result: OpaquePayload }
  | {
      readonly kind: 'remoteError';
      readonly code: number;
      readonly message: string;
      readonly dataPresent: boolean;
      readonly data?: OpaquePayload;
    };

export type PlatformRpcError =
  | { readonly kind: 'parse' }
  | { readonly kind: 'invalidRequest' }
  | { readonly kind: 'methodNotFound' }
  | { readonly kind: 'invalidParams' }
  | { readonly kind: 'internal' }
  | { readonly kind: 'serverBusy' }
  | { readonly kind: 'timeout' };

export type ProfileAction =
  | {
      readonly kind: 'request';
      readonly id: OpaquePeerId;
      readonly method: string;
      readonly params: OpaquePayload;
    }
  | {
      readonly kind: 'response';
      readonly id: OpaquePeerId;
      readonly terminal: ProfileResponse;
    }
  | {
      readonly kind: 'ignoredNotification';
      readonly method: string;
      readonly params?: OpaquePayload;
    }
  | {
      readonly kind: 'platformError';
      readonly id: OpaquePeerId | null;
      readonly error: PlatformRpcError;
    }
  | { readonly kind: 'close'; readonly code: number; readonly reason: string };

export interface WebSocketRpcProfileAdapter {
  readonly profile: ProfileId;

  classifyText(frame: string, limits: ProfileLimits): ProfileAction;
  peerIdKey(id: OpaquePeerId): string;
  nextOutboundId(generation: OutboundIdGeneration): OpaquePeerId;
  fromRuntimePayload(
    bytes: Uint8Array,
    purpose: 'outboundParams' | 'inboundResult',
    limits: ProfileLimits
  ): OpaquePayload;
  toRuntimePayload(
    payload: OpaquePayload,
    limits: ProfileLimits
  ): Uint8Array;
  encodeOutboundRequest(input: {
    readonly id: OpaquePeerId;
    readonly method: string;
    readonly params: OpaquePayload;
  }): string;
  encodeResult(id: OpaquePeerId, result: OpaquePayload): string;
  encodePlatformError(
    id: OpaquePeerId | null,
    error: PlatformRpcError
  ): string;
}
