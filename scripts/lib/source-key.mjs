import { createHash } from 'node:crypto';
import { constants as fsConstants } from 'node:fs';
import { access, lstat, mkdir, readFile, readlink, rename, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { spawn } from 'node:child_process';

const SCHEMA_VERSION = 1;
const SOURCE_KEY_PREFIX = 'skiff-source-key-v1:sha256:';

export function stableJsonSha256(value) {
  return sha256Text(stableJson(value));
}

export async function sha256File(file) {
  return sha256Buffer(await readFile(file));
}

export function sha256Text(text) {
  return sha256Buffer(Buffer.from(text));
}

export async function readJsonIfExists(file) {
  try {
    return JSON.parse(await readFile(file, 'utf8'));
  } catch (error) {
    if (error.code === 'ENOENT') {
      return undefined;
    }
    throw error;
  }
}

export async function writeJsonAtomic(file, value) {
  await mkdir(path.dirname(file), { recursive: true });
  const temporary = path.join(
    path.dirname(file),
    `.${path.basename(file)}.${process.pid}.${Date.now()}.tmp`,
  );
  try {
    await writeFile(temporary, `${stableJson(value)}\n`);
    await rename(temporary, file);
  } catch (error) {
    await rm(temporary, { force: true }).catch(() => {});
    throw error;
  }
}

export async function sourceKeyFromInputs({ repoRoot, component, inputs, extra } = {}) {
  if (!repoRoot) {
    throw new Error('sourceKeyFromInputs requires repoRoot');
  }
  if (!component) {
    throw new Error('sourceKeyFromInputs requires component');
  }
  if (!Array.isArray(inputs) || inputs.length === 0) {
    throw new Error('sourceKeyFromInputs requires a non-empty inputs array');
  }

  const absoluteRepoRoot = path.resolve(repoRoot);
  const gitRoot = await gitTopLevel(absoluteRepoRoot);
  const gitPrefix = toPosix(path.relative(gitRoot, absoluteRepoRoot));
  const commit = (await gitText(['rev-parse', 'HEAD'], gitRoot)).trim();
  const resolvedInputs = [];

  for (const input of inputs) {
    const relativeInput = normalizeInputPath(input);
    const gitPath = joinPosix(gitPrefix, relativeInput);
    const gitObject = await gitObjectAtCommit(gitRoot, commit, gitPath);
    const dirty = await isInputDirty(gitRoot, absoluteRepoRoot, relativeInput);
    const resolved = {
      path: relativeInput,
      gitObject,
      dirty,
    };
    if (dirty) {
      resolved.worktreeHash = await worktreeContentHash(absoluteRepoRoot, relativeInput);
    }
    resolvedInputs.push(resolved);
  }

  const source = {
    schemaVersion: SCHEMA_VERSION,
    component,
    commit,
    inputs: resolvedInputs,
    extra,
  };
  const sourceHash = stableJsonSha256(source);

  return {
    ...source,
    sourceHash,
    sourceKey: `${SOURCE_KEY_PREFIX}${sourceHash}`,
  };
}

async function gitObjectAtCommit(gitRoot, commit, gitPath) {
  try {
    return (await gitText(['rev-parse', `${commit}:${gitPath}`], gitRoot)).trim();
  } catch (error) {
    if (isMissingCommitPathError(error)) {
      return null;
    }
    throw error;
  }
}

function isMissingCommitPathError(error) {
  const message = String(error?.message || '');
  return message.includes('exists on disk, but not in')
    || message.includes('does not exist in');
}

export function componentStatePath(root, component) {
  return path.join(root, 'release-state', 'components', `${component}.json`);
}

async function isInputDirty(gitRoot, repoRoot, relativeInput) {
  const output = await gitBuffer(['status', '--porcelain=v1', '-z', '--', relativeInput], repoRoot);
  if (output.length > 0) {
    return true;
  }

  const treePath = joinPosix(toPosix(path.relative(gitRoot, repoRoot)), relativeInput);
  const diffQuiet = await gitExitCode(['diff', '--quiet', 'HEAD', '--', treePath], gitRoot);
  return diffQuiet !== 0;
}

async function worktreeContentHash(repoRoot, relativeInput) {
  const files = await gitWorktreeFiles(repoRoot, relativeInput);
  const entries = [];
  for (const file of files.sort((left, right) => left.localeCompare(right))) {
    entries.push(await collectWorktreeEntry(path.join(repoRoot, file), file));
  }
  return stableJsonSha256(entries);
}

async function collectWorktreeEntry(absolutePath, relativePath) {
  let stat;
  try {
    stat = await lstat(absolutePath);
  } catch (error) {
    if (error.code === 'ENOENT') {
      return { path: relativePath, type: 'missing' };
    }
    throw error;
  }

  if (stat.isSymbolicLink()) {
    return {
      path: relativePath,
      type: 'symlink',
      target: await readlink(absolutePath),
    };
  }

  if (stat.isFile()) {
    return {
      path: relativePath,
      type: 'file',
      executable: await isExecutable(absolutePath),
      sha256: await sha256File(absolutePath),
    };
  }

  return {
    path: relativePath,
    type: 'special',
    mode: (stat.mode & 0o7777).toString(8),
  };
}

async function gitWorktreeFiles(repoRoot, relativeInput) {
  const output = await gitBuffer([
    'ls-files',
    '-z',
    '--cached',
    '--others',
    '--exclude-standard',
    '--',
    relativeInput,
  ], repoRoot);
  return output
    .toString('utf8')
    .split('\0')
    .filter(Boolean);
}

async function isExecutable(file) {
  try {
    await access(file, fsConstants.X_OK);
    return true;
  } catch {
    return false;
  }
}

function normalizeInputPath(input) {
  if (typeof input !== 'string' || input.length === 0) {
    throw new Error(`invalid source input path: ${input}`);
  }
  const normalized = toPosix(path.normalize(input));
  if (normalized === '.' || normalized.startsWith('../') || path.isAbsolute(input)) {
    throw new Error(`source input path must be relative to repoRoot: ${input}`);
  }
  return normalized;
}

function stableJson(value) {
  return JSON.stringify(sortValue(value));
}

function sortValue(value) {
  if (Array.isArray(value)) {
    return value.map(sortValue);
  }
  if (!value || typeof value !== 'object') {
    return value;
  }
  return Object.fromEntries(
    Object.entries(value)
      .filter(([, nested]) => nested !== undefined)
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([key, nested]) => [key, sortValue(nested)]),
  );
}

function sha256Buffer(content) {
  return createHash('sha256').update(content).digest('hex');
}

function joinPosix(...parts) {
  return parts.filter(Boolean).join('/');
}

function toPosix(value) {
  return value.replaceAll(path.sep, '/');
}

async function gitTopLevel(cwd) {
  return (await gitText(['rev-parse', '--show-toplevel'], cwd)).trim();
}

function gitText(args, cwd) {
  return gitBuffer(args, cwd).then((buffer) => buffer.toString('utf8'));
}

function gitBuffer(args, cwd) {
  return new Promise((resolve, reject) => {
    const child = spawn('git', args, {
      cwd,
      env: process.env,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    const stdout = [];
    const stderr = [];
    child.stdout.on('data', (chunk) => stdout.push(chunk));
    child.stderr.on('data', (chunk) => stderr.push(chunk));
    child.on('error', reject);
    child.on('exit', (code, signal) => {
      if (code === 0) {
        resolve(Buffer.concat(stdout));
        return;
      }
      reject(new Error(`git ${args.join(' ')} failed with ${signal || code}: ${Buffer.concat(stderr).toString('utf8')}`));
    });
  });
}

function gitExitCode(args, cwd) {
  return new Promise((resolve, reject) => {
    const child = spawn('git', args, {
      cwd,
      env: process.env,
      stdio: ['ignore', 'ignore', 'pipe'],
    });
    const stderr = [];
    child.stderr.on('data', (chunk) => stderr.push(chunk));
    child.on('error', reject);
    child.on('exit', (code, signal) => {
      if (signal) {
        reject(new Error(`git ${args.join(' ')} failed with ${signal}: ${Buffer.concat(stderr).toString('utf8')}`));
        return;
      }
      resolve(code);
    });
  });
}
