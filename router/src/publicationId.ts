const MAX_PUBLICATION_ID_BYTES = 63;

const AUTHORITY_LABEL_PATTERN = '[a-z0-9](?:[a-z0-9-]*[a-z0-9])?';
const AUTHORITY_PATTERN = `${AUTHORITY_LABEL_PATTERN}\\.${AUTHORITY_LABEL_PATTERN}(?:\\.${AUTHORITY_LABEL_PATTERN})*`;
const LOCAL_SEGMENT_PATTERN = '[a-z](?:[a-z0-9_-]*[a-z0-9_])?';
const LOCAL_PATH_PATTERN = `${LOCAL_SEGMENT_PATTERN}(?:/${LOCAL_SEGMENT_PATTERN})*`;
const PUBLICATION_ID_PATTERN = new RegExp(`^${AUTHORITY_PATTERN}/${LOCAL_PATH_PATTERN}$`);

export function isPublicationId(value: string): boolean {
  return Buffer.byteLength(value, 'utf8') <= MAX_PUBLICATION_ID_BYTES &&
    value !== 'std' &&
    value.trim() === value &&
    !value.includes('~') &&
    PUBLICATION_ID_PATTERN.test(value);
}

export function assertPublicationId(publicationId: string, label = 'publication id'): void {
  if (!isPublicationId(publicationId)) {
    throw new Error(`${label} ${publicationId} must be a publication id`);
  }
}

export function publicationDisplayForm(publicationId: string): string {
  assertPublicationId(publicationId);
  return publicationId;
}

export function publicationAuthority(publicationId: string): string | undefined {
  assertPublicationId(publicationId);
  const authority = publicationId.split('/', 1)[0]!;
  return authority.includes('.') ? authority : undefined;
}

export function publicationStorageSegment(publicationId: string): string {
  assertPublicationId(publicationId);
  return publicationId.replaceAll('.', '~').replaceAll('/', '~~');
}
