import { lstat, readFile, readdir } from 'node:fs/promises';
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { parseYamlStringScalar } from './simple-yaml.mjs';

const controlFilePatterns = [
  /^package\.yml$/,
  /^service\.yml$/,
  /^service\.[^.]+\.yml$/,
  /^api\.yml$/,
  /^config\.yml$/,
  /^config\.[^.]+\.yml$/,
  /^.*\.secret\.yml$/,
];

export async function manifestDeclaredResourcePaths(manifestPath) {
  const text = await readFile(manifestPath, 'utf8');
  return parsePublicationResourceList(text, manifestPath);
}

export async function validatedManifestResourceArchivePaths(root, manifestPath) {
  const paths = await manifestDeclaredResourcePaths(manifestPath);
  const unique = new Set();
  const result = [];
  for (const path of paths) {
    validatePublicationResourceLogicalPath(path, manifestPath);
    if (unique.has(path)) {
      throw new Error(`${manifestPath} resources contains duplicate path ${path}`);
    }
    unique.add(path);
    await validatePublicationResourceFile(root, path, manifestPath);
    result.push(path);
  }
  return result;
}

export async function discoverDeclaredResourceFiles(root, manifestNames) {
  const result = [];
  await visitManifestDirectories(root, async (manifestPath) => {
    if (!manifestNames.has(basename(manifestPath))) {
      return;
    }
    const manifestRoot = dirname(manifestPath);
    for (const resourcePath of await validatedManifestResourceArchivePaths(manifestRoot, manifestPath)) {
      result.push(join(manifestRoot, ...resourcePath.split('/')));
    }
  });
  return result;
}

export function parsePublicationResourceList(text, label = 'manifest') {
  const lines = text.split(/\r?\n/);
  const resources = [];
  for (let index = 0; index < lines.length; index += 1) {
    const line = stripComment(lines[index]);
    if (/^\s*$/.test(line) || /^---\s*$/.test(line)) {
      continue;
    }
    const match = /^resources\s*:\s*(.*)$/.exec(line);
    if (!match) {
      continue;
    }
    const rawValue = match[1].trim();
    if (rawValue === '[]') {
      return [];
    }
    if (rawValue.length > 0) {
      throw new Error(`${label} resources must be a block list`);
    }
    const resourceIndent = leadingWhitespace(lines[index]).length;
    for (let itemIndex = index + 1; itemIndex < lines.length; itemIndex += 1) {
      const itemLine = stripComment(lines[itemIndex]);
      if (/^\s*$/.test(itemLine)) {
        continue;
      }
      const indent = leadingWhitespace(itemLine).length;
      if (indent <= resourceIndent) {
        break;
      }
      const itemMatch = /^\s*-\s+(.+?)\s*$/.exec(itemLine);
      if (!itemMatch) {
        throw new Error(`${label} resources must contain string path list items`);
      }
      resources.push(parseYamlStringScalar(itemMatch[1]));
    }
    return resources;
  }
  return [];
}

export function validatePublicationResourceLogicalPath(path, label = 'resource') {
  if (typeof path !== 'string' || path.length === 0) {
    throw new Error(`${label} resource path must not be empty`);
  }
  if (path !== path.trim() || path.includes('\0') || path.includes('\n') || path.includes('\r')) {
    throw new Error(`${label} resource path is invalid: ${path}`);
  }
  if (path === '.' || path === '..' || path.startsWith('/') || isAbsolute(path)) {
    throw new Error(`${label} resource path is unsafe: ${path}`);
  }
  if (path.includes('\\') || path.includes('//') || path.startsWith('./') || path.endsWith('/')) {
    throw new Error(`${label} resource path is not canonical: ${path}`);
  }
  const parts = path.split('/');
  if (parts.some((part) => part.length === 0 || part === '.' || part === '..')) {
    throw new Error(`${label} resource path is not canonical: ${path}`);
  }
  if (parts.some((part) => part.startsWith('.'))) {
    throw new Error(`${label} resource path must not include hidden path segments: ${path}`);
  }
  if (path.endsWith('.skiff') || controlFilePatterns.some((pattern) => pattern.test(path))) {
    throw new Error(`${label} resource path must not be a Skiff source or control file: ${path}`);
  }
  return path;
}

async function validatePublicationResourceFile(root, logicalPath, label) {
  const absoluteRoot = resolve(root);
  const exactPath = await exactCasePath(absoluteRoot, logicalPath, label);
  const relativePath = relative(absoluteRoot, exactPath).split(sep).join('/');
  if (relativePath !== logicalPath) {
    throw new Error(`${label} resource path case mismatch: ${logicalPath}`);
  }
  const metadata = await lstat(exactPath);
  if (metadata.isSymbolicLink()) {
    throw new Error(`${label} resource path must not be a symlink: ${logicalPath}`);
  }
  if (!metadata.isFile()) {
    throw new Error(`${label} resource path must be a regular file: ${logicalPath}`);
  }
}

async function exactCasePath(root, logicalPath, label) {
  let current = root;
  for (const segment of logicalPath.split('/')) {
    const entries = await readdir(current, { withFileTypes: true });
    const entry = entries.find((candidate) => candidate.name === segment);
    if (!entry) {
      throw new Error(`${label} resource path does not exist with exact case: ${logicalPath}`);
    }
    current = join(current, segment);
  }
  return current;
}

async function visitManifestDirectories(root, visit) {
  async function walk(directory) {
    const entries = await readdir(directory, { withFileTypes: true }).catch((error) => {
      if (error?.code === 'ENOENT') {
        return [];
      }
      throw error;
    });
    for (const entry of entries) {
      const entryPath = join(directory, entry.name);
      if (entry.isDirectory()) {
        if (!entry.name.startsWith('.') && entry.name !== 'node_modules') {
          await walk(entryPath);
        }
      } else if (entry.isFile() && (entry.name === 'package.yml' || entry.name === 'service.yml')) {
        await visit(entryPath);
      }
    }
  }
  await walk(resolve(root));
}

function stripComment(line) {
  const hash = line.indexOf('#');
  return hash === -1 ? line : line.slice(0, hash);
}

function leadingWhitespace(line) {
  return line.match(/^\s*/)?.[0] ?? '';
}
