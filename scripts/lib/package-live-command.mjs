import { captureCheckedCommand } from './command-execution.mjs';

export function createPackageLiveCommand({
  skiffCli,
  cwd,
  env = process.env,
  nodeCommand = process.execPath,
  checkedRunner = captureCheckedCommand,
} = {}) {
  if (typeof skiffCli !== 'string' || skiffCli.length === 0) {
    throw new Error('package live command requires skiffCli');
  }

  async function runCli(args) {
    let result;
    try {
      result = await checkedRunner(
        nodeCommand,
        [skiffCli, ...args],
        { cwd, env },
      );
    } catch (error) {
      throw rebuildPackageCommandError(args, error);
    }
    return { stdout: result.stdout.trim(), stderr: result.stderr.trim() };
  }

  async function runCliJson(args) {
    const { stdout } = await runCli(args);
    try {
      return JSON.parse(stdout);
    } catch (error) {
      throw new Error(
        `${packageCommandLabel(args)} returned invalid JSON: ${safeMessage(error)}\nstdout:\n${stdout}`,
      );
    }
  }

  return Object.freeze({ runCli, runCliJson });
}

function rebuildPackageCommandError(args, error) {
  const status = error?.signal ?? error?.code ?? 'UNKNOWN';
  return new Error([
    `${packageCommandLabel(args)} exited with ${status}`,
    streamDiagnostic('stderr', error?.stderr),
    streamDiagnostic('stdout', error?.stdout),
  ].filter(Boolean).join('\n'));
}

function packageCommandLabel(args) {
  const safeSubcommand = args[0] === 'package' && typeof args[1] === 'string'
    ? `package ${args[1]}`
    : 'package command';
  return `skiff ${safeSubcommand}`;
}

function streamDiagnostic(label, value) {
  return typeof value === 'string' && value.trim().length > 0
    ? `${label}:\n${value.trim()}`
    : '';
}

function safeMessage(error) {
  return typeof error?.message === 'string' ? error.message : String(error);
}
