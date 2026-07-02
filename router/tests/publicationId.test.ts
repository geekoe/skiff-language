import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

import {
  isPublicationId,
  publicationAuthority,
  publicationDisplayForm,
  publicationStorageSegment
} from '../src/publicationId.js';

type PublicationIdCase = {
  canonicalId: string;
  displayForm: string;
  authority: string;
  runtimeTargetComponent: string;
  appliesTo: string[];
};

type InvalidPublicationIdCase = {
  id: string;
  appliesTo: string[];
};

type PublicationIdFixture = {
  schemaVersion: number;
  encoding: string;
  maxBytes: number;
  valid: PublicationIdCase[];
  invalid: InvalidPublicationIdCase[];
};

const publicationIdFixture = JSON.parse(
  readFileSync(
    new URL('../../cross-system-fixtures/publication-id-cases.json', import.meta.url),
    'utf8'
  )
) as PublicationIdFixture;

function appliesToRouter(item: { appliesTo: string[] }): boolean {
  return item.appliesTo.includes('router');
}

describe('publication ids', () => {
  it('matches the shared publication id fixture', () => {
    expect(publicationIdFixture.schemaVersion).toBe(1);
    expect(publicationIdFixture.encoding).toBe('url-like-with-storage-safe-projection');
    expect(publicationIdFixture.maxBytes).toBe(63);

    for (const item of publicationIdFixture.valid.filter(appliesToRouter)) {
      expect(isPublicationId(item.canonicalId), item.canonicalId).toBe(true);
      expect(publicationDisplayForm(item.canonicalId)).toBe(item.displayForm);
      expect(publicationAuthority(item.canonicalId)).toBe(item.authority);
      expect(publicationStorageSegment(item.canonicalId)).toBe(item.runtimeTargetComponent);
    }

    for (const item of publicationIdFixture.invalid.filter(appliesToRouter)) {
      expect(isPublicationId(item.id), item.id).toBe(false);
    }
  });

  it('accepts url-like publication ids', () => {
    for (const value of [
      'skiff.run/http_session',
      'api.skiff.run/chat',
      'example.com/billing_worker',
      'skiff.run/std',
      'skiff.run/package/nested-service',
      'example.com/a--b',
      'example.com/a-_b'
    ]) {
      expect(isPublicationId(value), value).toBe(true);
    }
  });

  it('rejects storage-safe projections, unsafe characters, and overlong ids', () => {
    for (const value of [
      'skiff~run',
      'std',
      'billing',
      'skiff.run',
      'skiff~run~~std',
      'skiff.run/9cloud',
      'skiff.run/cloud-',
      'skiff~run~~cloud/api',
      'skiff.run//cloud',
      'skiff.run/',
      'skiff.run/' + 'a'.repeat(60)
    ]) {
      expect(isPublicationId(value), value).toBe(false);
    }
  });

  it('keeps display form semantic and projects storage separately', () => {
    expect(publicationDisplayForm('api.skiff.run/chat')).toBe('api.skiff.run/chat');
    expect(publicationAuthority('api.skiff.run/chat')).toBe('api.skiff.run');
    expect(publicationStorageSegment('api.skiff.run/chat')).toBe('api~skiff~run~~chat');
  });
});
