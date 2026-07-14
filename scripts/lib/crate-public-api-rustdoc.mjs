import { constants } from 'node:fs';
import { access, readFile } from 'node:fs/promises';
import { spawn as spawnRustdocChild } from 'node:child_process';
import { join } from 'node:path';

export const NIGHTLY_PROBE_TIMEOUT_MS = 10_000;

export async function cargoMetadata({ env, root, runCommand: execute = runCommand }) {
  const result = await execute(
    'cargo',
    ['metadata', '--format-version', '1', '--no-deps'],
    { cwd: root, env },
  );
  try {
    return JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(`failed to parse cargo metadata JSON: ${error.message}`);
  }
}

export async function probeCargoNightly({ env, root, runCommand: execute = runCommand }) {
  try {
    await execute('cargo', ['+nightly', '--version'], {
      cwd: root,
      env,
      timeoutMs: NIGHTLY_PROBE_TIMEOUT_MS,
    });
    return { available: true };
  } catch (error) {
    return { available: false, error };
  }
}

export async function buildRustdocJson({
  crateName,
  env,
  nightlyProbe,
  root,
  runCommand: execute = runCommand,
}) {
  const attempts = [];
  if (nightlyProbe.available) {
    attempts.push(rustdocJsonCommand(crateName, { env, nightly: true, root }));
  }
  attempts.push(rustdocJsonCommand(crateName, { env, nightly: false, root }));

  const failures = [];
  for (const attempt of attempts) {
    try {
      await execute(attempt.command, attempt.args, attempt.options);
      return {
        fallbackLabel: failures.length > 0 ? attempt.label : undefined,
      };
    } catch (error) {
      failures.push({ attempt, error });
    }
  }

  const detailParts = [];
  if (!nightlyProbe.available && nightlyProbe.error) {
    detailParts.push(
      `nightly probe failed; skipped cargo +nightly rustdoc:\n${commandFailureDetail(
        nightlyProbe.error,
      )}`,
    );
  }
  for (const failure of failures) {
    detailParts.push(`${failure.attempt.label} failed:\n${commandFailureDetail(failure.error)}`);
  }
  throw new Error(
    `failed to build rustdoc JSON for ${crateName}. This crate exists, so rustdoc JSON support is a blocking failure.\n${detailParts.join(
      '\n\n',
    )}`,
  );
}

export async function readRustdocJson({ metadata, packageInfo }) {
  const path = rustdocJsonPath(metadata, packageInfo);
  await assertReadable(path);
  return JSON.parse(await readFile(path, 'utf8'));
}

export function rustdocJsonCommand(crateName, { env, nightly, root }) {
  const args = [
    ...(nightly ? ['+nightly'] : []),
    'rustdoc',
    '-p',
    crateName,
    '--lib',
    '--',
    '-Z',
    'unstable-options',
    '--output-format',
    'json',
  ];
  const options = { cwd: root };
  if (!nightly) {
    options.env = { ...env, RUSTC_BOOTSTRAP: '1' };
  }
  return {
    args,
    command: 'cargo',
    label: nightly ? 'cargo +nightly rustdoc' : 'RUSTC_BOOTSTRAP=1 cargo rustdoc',
    options,
  };
}

export function rustdocJsonPath(metadata, packageInfo) {
  const libTarget = packageInfo.targets.find((target) => target.kind.includes('lib'));
  if (!libTarget) {
    throw new Error(`${packageInfo.name} exists but has no lib target to document`);
  }
  return join(metadata.target_directory, 'doc', `${rustdocFileStem(libTarget.name)}.json`);
}

export function runCommand(command, args, options = {}) {
  return runCommandWithOwnedChild(
    // child-process-owner: rustdoc-timeout
    () => spawnRustdocChild(command, args, childOptions(options)),
    command,
    args,
    options,
    { clearTimer: clearTimeout, setTimer: setTimeout },
  );
}

export function createRustdocCommandRunner({
  clearTimer,
  createChild,
  setTimer,
}) {
  return (command, args, options = {}) => runCommandWithOwnedChild(
    () => createChild(command, args, childOptions(options)),
    command,
    args,
    options,
    { clearTimer, setTimer },
  );
}

function runCommandWithOwnedChild(createChild, command, args, options, timers) {
  return new Promise((resolve, reject) => {
    const child = createChild();
    let stdout = '';
    let stderr = '';
    let settled = false;
    let timedOut = false;
    let timeout;

    if (options.timeoutMs) {
      timeout = timers.setTimer(() => {
        timedOut = true;
        child.kill('SIGKILL');
      }, options.timeoutMs);
    }

    function complete(callback) {
      if (settled) {
        return;
      }
      settled = true;
      if (timeout) {
        timers.clearTimer(timeout);
      }
      callback();
    }

    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', (chunk) => {
      stdout += chunk;
    });
    child.stderr.on('data', (chunk) => {
      stderr += chunk;
    });
    child.on('error', (error) => {
      error.command = formatCommand(command, args);
      error.stdout = stdout;
      error.stderr = stderr;
      complete(() => reject(error));
    });
    child.on('close', (code, signal) => {
      if (code === 0) {
        complete(() => resolve({ stdout, stderr }));
        return;
      }
      const message = timedOut
        ? `${formatCommand(command, args)} timed out after ${options.timeoutMs}ms`
        : `${formatCommand(command, args)} exited with ${code ?? signal}`;
      const error = new Error(message);
      error.command = formatCommand(command, args);
      error.exitCode = code;
      error.signal = signal;
      error.stdout = stdout;
      error.stderr = stderr;
      error.timedOut = timedOut;
      error.timeoutMs = timedOut ? options.timeoutMs : undefined;
      complete(() => reject(error));
    });
  });
}

function childOptions(options) {
  return {
    cwd: options.cwd,
    env: options.env,
    stdio: ['ignore', 'pipe', 'pipe'],
  };
}

function rustdocFileStem(targetName) {
  return targetName.replaceAll('-', '_');
}

async function assertReadable(path) {
  try {
    await access(path, constants.R_OK);
  } catch (error) {
    if (error && error.code === 'ENOENT') {
      throw new Error(`rustdoc JSON was not produced at ${path}`);
    }
    throw error;
  }
}

function commandFailureDetail(error) {
  const parts = [`command: ${error.command ?? 'unknown'}`];
  if (error.message) {
    parts.push(`error: ${error.message}`);
  }
  if (error.stderr) {
    parts.push(`stderr:\n${error.stderr.trimEnd()}`);
  }
  if (error.stdout) {
    parts.push(`stdout:\n${error.stdout.trimEnd()}`);
  }
  return parts.join('\n');
}

function formatCommand(command, args) {
  return [command, ...args].join(' ');
}
