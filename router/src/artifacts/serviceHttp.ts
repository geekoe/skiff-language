import { isRecord } from "./readUtils.js";

export function serviceHttpHashInput(
  service: Record<string, unknown>,
  label: string,
): Record<string, unknown> | undefined {
  if (!Object.prototype.hasOwnProperty.call(service, "http")) {
    return undefined;
  }
  const http = service.http;
  if (!isRecord(http)) {
    throw new Error(`${label}.http must be an object`);
  }
  rejectUnsupportedKeys(http, ["response"], `${label}.http`);

  const response = http.response;
  if (!isRecord(response)) {
    throw new Error(`${label}.http.response must be an object`);
  }
  rejectUnsupportedKeys(response, ["maxBytes"], `${label}.http.response`);
  if (!Object.prototype.hasOwnProperty.call(response, "maxBytes")) {
    throw new Error(
      `${label}.http.response.maxBytes is required when service.http is present`,
    );
  }
  const maxBytes = response.maxBytes;
  if (
    typeof maxBytes !== "number" ||
    !Number.isSafeInteger(maxBytes) ||
    maxBytes <= 0
  ) {
    throw new Error(
      `${label}.http.response.maxBytes must be a positive integer`,
    );
  }

  return {
    http: {
      response: {
        maxBytes,
      },
    },
  };
}

function rejectUnsupportedKeys(
  value: Record<string, unknown>,
  supported: readonly string[],
  label: string,
): void {
  const unsupported = Object.keys(value).filter(
    (key) => !supported.includes(key),
  );
  if (unsupported.length > 0) {
    throw new Error(
      `${label} does not support ${unsupported.map((key) => `${label}.${key}`).join(", ")}`,
    );
  }
}
