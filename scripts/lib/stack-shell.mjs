import { captureCheckedCommand, runAttachedCommand } from './command-execution.mjs';

/**
 * Remote shell boundary shared by stack deploy/status/init.
 *
 * Commands are injectable so focused tests can record calls without a real
 * remote host; the production default runs attached ssh/rsync from the skiff
 * repo root with the caller's environment.
 */
export function createStackShell({
  skiffRoot,
  runCommand = runAttachedCommand,
  captureCommand = captureCheckedCommand,
  env = process.env,
} = {}) {
  const options = { cwd: skiffRoot, env };
  return Object.freeze({
    remoteRun: async (host, command) => {
      await runCommand('ssh', [host, command], options);
    },
    remoteCapture: async (host, command) => {
      const outcome = await captureCommand('ssh', [host, command], options);
      return outcome.stdout;
    },
    rsync: async (source, destination, extra = []) => {
      await runCommand('rsync', ['-az', ...extra, source, destination], options);
    },
  });
}
