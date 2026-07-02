const REVISION_ID_PATTERN = /^[0-9a-f]{64}$/;

export function isRevisionId(value: string): boolean {
  return REVISION_ID_PATTERN.test(value);
}

export function assertRevisionId(value: string, name: string): void {
  if (!isRevisionId(value)) {
    throw new Error(`${name} must be <64 lowercase hex>`);
  }
}
