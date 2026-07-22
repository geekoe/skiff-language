export function throwPrimaryWithCleanup(
  primary: unknown,
  cleanup: readonly unknown[],
  message: string
): never {
  if (cleanup.length > 0) {
    throw new AggregateError([primary, ...cleanup], message, { cause: primary });
  }
  throw primary;
}

export function throwCleanupErrors(
  cleanup: readonly unknown[],
  message: string
): void {
  if (cleanup.length === 1) {
    throw cleanup[0];
  }
  if (cleanup.length > 1) {
    throw new AggregateError(cleanup, message);
  }
}
