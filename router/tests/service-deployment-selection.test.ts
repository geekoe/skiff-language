import { describe, expect, it } from 'vitest';

import { readServiceDeploymentSelector } from '../src/router/serviceDeploymentSelection.js';

describe('trusted service deployment selection', () => {
  it('reads one exact canonical service/version pair', () => {
    expect(readServiceDeploymentSelector({
      headers: {
        'x-skiff-service': 'example.com/chat',
        'x-skiff-version': '1.0.0'
      }
    })).toEqual({
      serviceId: 'example.com/chat',
      contractVersion: '1.0.0'
    });
  });

  it.each([
    {
      name: 'missing service',
      headers: { 'x-skiff-version': '1.0.0' },
      code: 'ServiceSelectorRequired'
    },
    {
      name: 'missing version',
      headers: { 'x-skiff-service': 'example.com/chat' },
      code: 'VersionSelectorRequired'
    },
    {
      name: 'ambiguous service',
      headers: {
        'x-skiff-service': 'example.com/chat,example.com/other',
        'x-skiff-version': '1.0.0'
      },
      code: 'ServiceSelectorInvalid'
    },
    {
      name: 'non-canonical version',
      headers: {
        'x-skiff-service': 'example.com/chat',
        'x-skiff-version': ' 1.0.0'
      },
      code: 'VersionSelectorInvalid'
    }
  ])('fails closed for $name', ({ headers, code }) => {
    expect(() => readServiceDeploymentSelector({ headers })).toThrow(
      expect.objectContaining({ statusCode: 400, code })
    );
  });
});
