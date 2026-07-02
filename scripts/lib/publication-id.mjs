const publicationAuthorityLabelPattern = '[a-z0-9](?:[a-z0-9-]*[a-z0-9])?';
const publicationAuthorityPattern = `${publicationAuthorityLabelPattern}\\.${publicationAuthorityLabelPattern}(?:\\.${publicationAuthorityLabelPattern})*`;
const publicationLocalSegmentPattern = '[a-z](?:[a-z0-9_-]*[a-z0-9_])?';
const publicationIdPattern = new RegExp(
  `^${publicationAuthorityPattern}/${publicationLocalSegmentPattern}(?:/${publicationLocalSegmentPattern})*$`
);

export function isPublicationId(publicationId) {
  return typeof publicationId === 'string'
    && Buffer.byteLength(publicationId, 'utf8') <= 63
    && publicationId !== 'std'
    && publicationId.trim() === publicationId
    && !publicationId.includes('~')
    && publicationIdPattern.test(publicationId);
}

export function assertPublicationId(publicationId, label) {
  if (!isPublicationId(publicationId)) {
    throw new Error(`${label} must be a publication id`);
  }
}

export function publicationStorageSegment(publicationId) {
  return publicationId.replaceAll('.', '~').replaceAll('/', '~~');
}

export function publicationStoragePathSegments(publicationId, label = 'publication id') {
  assertPublicationId(publicationId, label);
  return [publicationStorageSegment(publicationId)];
}
