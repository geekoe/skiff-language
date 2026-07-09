export const REQUEST_CANCEL_REASONS = [
  'timeout',
  'caller_cancel',
  'runtime_disconnect',
  'gateway_disconnect',
  'drain',
  'retire',
  'client_disconnect',
  'router_shutdown',
  'backpressure',
  'deadline_exceeded',
  'protocol_error',
  'stream_dropped'
] as const;

export type RequestCancelReason = (typeof REQUEST_CANCEL_REASONS)[number];

export const CONTRACT_H_REQUEST_CANCEL_SITUATIONS = [
  'caller_abort',
  'client_disconnect',
  'timeout',
  'deadline_exceeded',
  'backpressure',
  'protocol_error',
  'stream_dropped',
  'runtime_disconnect',
  'router_shutdown'
] as const;

export type RequestCancelSituation = (typeof CONTRACT_H_REQUEST_CANCEL_SITUATIONS)[number];

export const REQUEST_CANCEL_SITUATION = {
  callerAbort: 'caller_abort',
  clientDisconnect: 'client_disconnect',
  timeout: 'timeout',
  deadlineExceeded: 'deadline_exceeded',
  backpressure: 'backpressure',
  protocolError: 'protocol_error',
  streamDropped: 'stream_dropped',
  runtimeDisconnect: 'runtime_disconnect',
  routerShutdown: 'router_shutdown'
} as const satisfies Record<string, RequestCancelSituation>;

export const REQUEST_CANCEL_REASON_BY_SITUATION = {
  caller_abort: 'caller_cancel',
  client_disconnect: 'client_disconnect',
  timeout: 'timeout',
  deadline_exceeded: 'deadline_exceeded',
  backpressure: 'backpressure',
  protocol_error: 'protocol_error',
  stream_dropped: 'stream_dropped',
  runtime_disconnect: 'runtime_disconnect',
  router_shutdown: 'router_shutdown'
} as const satisfies Record<RequestCancelSituation, RequestCancelReason>;

export interface RequestCancelReasonMapping {
  internalReason: string;
  wireReason: RequestCancelReason;
}

const KNOWN_REQUEST_CANCEL_REASONS = new Set<string>(REQUEST_CANCEL_REASONS);

const INTERNAL_REASON_FALLBACKS = {
  caller_abort: REQUEST_CANCEL_REASON_BY_SITUATION.caller_abort,
  unexpected_stream_response: REQUEST_CANCEL_REASON_BY_SITUATION.protocol_error,
  unexpected_control_response: REQUEST_CANCEL_REASON_BY_SITUATION.protocol_error,
  response_channel_closed: REQUEST_CANCEL_REASON_BY_SITUATION.protocol_error,
  duplicate_response_start: REQUEST_CANCEL_REASON_BY_SITUATION.protocol_error,
  chunk_before_start: REQUEST_CANCEL_REASON_BY_SITUATION.protocol_error,
  chunk_seq_mismatch: REQUEST_CANCEL_REASON_BY_SITUATION.protocol_error,
  chunk_decode_error: REQUEST_CANCEL_REASON_BY_SITUATION.protocol_error,
  stream_end_payload: REQUEST_CANCEL_REASON_BY_SITUATION.protocol_error,
  stream_cancelled: REQUEST_CANCEL_REASON_BY_SITUATION.stream_dropped
} as const satisfies Record<string, RequestCancelReason>;

export function isRequestCancelReason(reason: string): reason is RequestCancelReason {
  return KNOWN_REQUEST_CANCEL_REASONS.has(reason);
}

export function requestCancelReasonForSituation(
  situation: RequestCancelSituation
): RequestCancelReason {
  return REQUEST_CANCEL_REASON_BY_SITUATION[situation];
}

export function mapInternalRequestCancelReason(internalReason: string): RequestCancelReasonMapping {
  if (isRequestCancelReason(internalReason)) {
    return { internalReason, wireReason: internalReason };
  }
  return {
    internalReason,
    wireReason:
      INTERNAL_REASON_FALLBACKS[internalReason as keyof typeof INTERNAL_REASON_FALLBACKS] ??
      REQUEST_CANCEL_REASON_BY_SITUATION.caller_abort
  };
}
