import {
  DEFAULT_JSON_RPC_20_TEXT_LIMITS,
  type OpaquePayload,
  type OpaquePeerId,
  type ProfileId,
  type ProfileLimits,
  type WebSocketRpcProfileAdapter
} from '../protocol/jsonRpc20TextProfile.js';

export interface CapturedPeerWriter {
  writeText(frame: string): void | Promise<void>;
  close(code: number, reason: string): void;
}

export type BrokerConnectionResponseOutcome =
  | 'success'
  | 'deadlineExceeded'
  | 'connectionUnavailable'
  | 'transportUnavailable'
  | 'protocolError'
  | 'resourceLimit'
  | 'remote';

export interface BrokerRuntimeResponse {
  readonly requestId: string;
  readonly outcome: BrokerConnectionResponseOutcome;
  readonly remote?: {
    readonly code: number;
    readonly message: string;
    readonly dataPresent: boolean;
  };
  readonly payloadBytes?: Uint8Array;
}

export interface BrokerRuntimeSource {
  readonly sender: object;
  readonly sessionToken: string;
  respond(response: BrokerRuntimeResponse): void | Promise<void>;
}

export interface BrokerConnectionGeneration {
  readonly connectionId: string;
  readonly socketGeneration: string;
  readonly serviceId: string;
  readonly websocketEntryId: string;
  readonly profile: ProfileId;
}

export interface InboundExecutionToken {
  readonly connectionId: string;
  readonly socketGeneration: string;
  readonly sequence: number;
}

export interface InboundDispatchAction {
  readonly profile: ProfileId;
  readonly connectionId: string;
  readonly socketGeneration: string;
  readonly peerId: OpaquePeerId;
  readonly method: string;
  readonly params: OpaquePayload;
  readonly executionToken: InboundExecutionToken;
  readonly signal: AbortSignal;
}

export interface InboundNotificationAction {
  readonly profile: ProfileId;
  readonly connectionId: string;
  readonly socketGeneration: string;
  readonly method: string;
  readonly params?: OpaquePayload;
}

export type InboundDispatchResult =
  | { readonly kind: 'success'; readonly result: OpaquePayload }
  | { readonly kind: 'invalidParams' }
  | { readonly kind: 'internalError' }
  | { readonly kind: 'deadlineExceeded' }
  | { readonly kind: 'runtimeUnavailable' };

export interface WebSocketRequestBrokerClock {
  now(): number;
  setTimeout(callback: () => void, delayMs: number): unknown;
  clearTimeout(handle: unknown): void;
}

export interface WebSocketRequestBrokerLimits {
  readonly profileLimits: ProfileLimits;
  readonly outboundGlobalCapacity: number;
  readonly outboundPerGenerationCapacity: number;
  readonly inboundGlobalCapacity: number;
  readonly inboundPerGenerationCapacity: number;
  readonly outboundTombstoneCapacity: number;
  readonly inboundTombstoneCapacity: number;
  readonly outboundTombstoneTtlMs: number;
  readonly inboundTombstoneTtlMs: number;
  readonly inboundTimeoutMs: number;
}

export const DEFAULT_WEB_SOCKET_REQUEST_BROKER_LIMITS:
  WebSocketRequestBrokerLimits = Object.freeze({
    profileLimits: DEFAULT_JSON_RPC_20_TEXT_LIMITS,
    outboundGlobalCapacity: 4_096,
    outboundPerGenerationCapacity: 128,
    inboundGlobalCapacity: 4_096,
    inboundPerGenerationCapacity: 128,
    outboundTombstoneCapacity: 4_096,
    inboundTombstoneCapacity: 4_096,
    outboundTombstoneTtlMs: 60_000,
    inboundTombstoneTtlMs: 60_000,
    inboundTimeoutMs: 120_000
  });

export interface WebSocketRequestBrokerOptions
  extends WebSocketRequestBrokerLimits {
  readonly profiles: readonly WebSocketRpcProfileAdapter[];
  readonly clock?: WebSocketRequestBrokerClock;
  readonly dispatchInbound: (
    action: InboundDispatchAction
  ) => InboundDispatchResult | Promise<InboundDispatchResult>;
  readonly observeNotification?: (action: InboundNotificationAction) => void;
  readonly onRuntimeProtocolViolation?: (
    source: BrokerRuntimeSource,
    reason: string
  ) => void;
}

export interface AttachBrokerGenerationOptions {
  readonly connectionId: string;
  readonly socketGeneration: string;
  readonly serviceId: string;
  readonly websocketEntryId: string;
  readonly ownerToken: unknown;
  readonly profile: ProfileId;
  readonly profileAdapter: WebSocketRpcProfileAdapter;
  readonly inboundTimeoutMs: number;
  readonly outboundIdPrefix: string;
  readonly writer: CapturedPeerWriter;
  readonly acceptInboundMethod?: (method: string) => boolean;
}

export interface BrokerRuntimeRequest {
  readonly source: BrokerRuntimeSource;
  readonly requestId: string;
  readonly serviceId: string;
  readonly websocketEntryId: string;
  readonly ownerToken: unknown;
  readonly profile: ProfileId;
  readonly method: string;
  readonly payloadBytes: Uint8Array;
  readonly deadlineAtMs?: number;
}

export interface WebSocketRequestBrokerSnapshot {
  readonly generationCount: number;
  readonly outboundPeerEntries: number;
  readonly outboundRuntimeEntries: number;
  readonly inboundActiveEntries: number;
  readonly outboundGenerationActive: number;
  readonly inboundGenerationActive: number;
  readonly outboundTombstones: number;
  readonly inboundTombstones: number;
  readonly timerCount: number;
  readonly terminalLeaseCount: number;
}
