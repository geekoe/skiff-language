import type {
  OpaquePayload,
  OpaquePeerId,
  PlatformRpcError,
  ProfileLimits,
  ProfileResponse,
  WebSocketRpcProfileAdapter
} from '../protocol/jsonRpc20TextProfileContracts.js';
import type {
  BrokerRuntimeResponse,
  InboundDispatchResult
} from './webSocketRequestBrokerTypes.js';

export type InboundTerminal =
  | { readonly kind: 'success'; readonly result: OpaquePayload }
  | PlatformRpcError;

export interface InboundTerminalPlan {
  readonly terminal: InboundTerminal;
  readonly abort: boolean;
}

export function materializeOutboundPeerParams(input: {
  readonly adapter: WebSocketRpcProfileAdapter;
  readonly payloadBytes: Uint8Array;
  readonly limits: ProfileLimits;
}): OpaquePayload {
  return input.adapter.fromRuntimePayload(
    input.payloadBytes,
    'outboundParams',
    input.limits
  );
}

export function encodeOutboundPeerRequest(input: {
  readonly adapter: WebSocketRpcProfileAdapter;
  readonly id: OpaquePeerId;
  readonly method: string;
  readonly params: OpaquePayload;
}): string {
  return input.adapter.encodeOutboundRequest({
    id: input.id,
    method: input.method,
    params: input.params
  });
}

export function mapPeerTerminalToRuntimeResponse(input: {
  readonly adapter: WebSocketRpcProfileAdapter;
  readonly limits: ProfileLimits;
  readonly requestId: string;
  readonly terminal: ProfileResponse;
}): BrokerRuntimeResponse {
  if (input.terminal.kind === 'success') {
    return {
      requestId: input.requestId,
      outcome: 'success',
      payloadBytes: input.adapter.toRuntimePayload(
        input.terminal.result,
        input.limits
      )
    };
  }
  return {
    requestId: input.requestId,
    outcome: 'remote',
    remote: {
      code: input.terminal.code,
      message: input.terminal.message,
      dataPresent: input.terminal.dataPresent
    },
    ...(input.terminal.data === undefined
      ? {}
      : {
          payloadBytes: input.adapter.toRuntimePayload(
            input.terminal.data,
            input.limits
          )
        })
  };
}

export function mapInboundDispatchResultToTerminal(
  result: InboundDispatchResult
): InboundTerminalPlan | undefined {
  switch (result.kind) {
    case 'success':
      return {
        terminal: { kind: 'success', result: result.result },
        abort: false
      };
    case 'invalidParams':
      return { terminal: { kind: 'invalidParams' }, abort: false };
    case 'internalError':
    case 'runtimeUnavailable':
      return { terminal: { kind: 'internal' }, abort: false };
    case 'deadlineExceeded':
      return { terminal: { kind: 'timeout' }, abort: true };
  }
  return undefined;
}

export function encodeInboundTerminalFrame(input: {
  readonly adapter: WebSocketRpcProfileAdapter;
  readonly id: OpaquePeerId;
  readonly terminal: InboundTerminal;
}): string {
  try {
    return input.terminal.kind === 'success'
      ? input.adapter.encodeResult(input.id, input.terminal.result)
      : input.adapter.encodePlatformError(input.id, input.terminal);
  } catch {
    return input.adapter.encodePlatformError(
      input.id,
      { kind: 'internal' }
    );
  }
}
