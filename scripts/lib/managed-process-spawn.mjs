export const managedMongoOpenFileLimit = 65_536;

export function managedProcessSpawnInvocation(spec, {
  platform = process.platform,
} = {}) {
  if (spec.name !== 'mongo' || platform === 'win32') {
    return {
      command: spec.command,
      args: spec.args,
    };
  }
  return {
    command: '/bin/sh',
    args: [
      '-c',
      'ulimit -n "$1" && shift && exec "$@"',
      'skiff-managed-mongo',
      String(managedMongoOpenFileLimit),
      spec.command,
      ...spec.args,
    ],
  };
}
