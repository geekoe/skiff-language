import type { IncomingMessage } from 'node:http';

import { GatewayError } from './errors.js';

export interface ServiceDeploymentSelector {
  serviceId: string;
  contractVersion: string;
}

export function readServiceDeploymentSelector(
  request: Pick<IncomingMessage, 'headers'>
): ServiceDeploymentSelector {
  return {
    serviceId: readRequiredSelectorHeader(
      request.headers['x-skiff-service'],
      'X-Skiff-Service',
      'ServiceSelectorRequired',
      'ServiceSelectorInvalid'
    ),
    contractVersion: readRequiredSelectorHeader(
      request.headers['x-skiff-version'],
      'X-Skiff-Version',
      'VersionSelectorRequired',
      'VersionSelectorInvalid'
    )
  };
}

function readRequiredSelectorHeader(
  input: string | string[] | undefined,
  headerName: string,
  missingCode: string,
  invalidCode: string
): string {
  if (input === undefined) {
    throw new GatewayError(400, missingCode, `${headerName} is required`);
  }
  if (Array.isArray(input) || input.includes(',')) {
    throw new GatewayError(400, invalidCode, `${headerName} must be singular`);
  }
  const value = input.trim();
  if (
    value.length === 0 ||
    value !== input ||
    /[\s\p{Cc}]/u.test(value)
  ) {
    throw new GatewayError(
      400,
      invalidCode,
      `${headerName} must be a non-empty canonical token`
    );
  }
  return value;
}
