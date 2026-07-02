import { readFile, stat } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { parseYamlStringScalar, yamlStringScalarHasContent } from './simple-yaml.mjs';

export const projectConfigFile = 'skiff.yml';
export const localProjectConfigFile = 'skiff.local.yml';
export const defaultProjectPackageDir = '.skiff-package-store';

export async function readProjectPackageDirs(startPath = process.cwd()) {
  const project = await findProjectConfig(startPath);
  if (!project) {
    return {
      configPath: undefined,
      configPaths: [],
      packageDirs: [],
      projectRoot: undefined,
    };
  }

  const baseConfig = project.baseConfigPath === undefined
    ? { hasPackageDirs: false, packageDirs: [] }
    : parseProjectConfig(await readFile(project.baseConfigPath, 'utf8'), project.baseConfigPath);
  const localConfig = project.localConfigPath === undefined
    ? { hasPackageDirs: false, packageDirs: [] }
    : parseProjectConfig(await readFile(project.localConfigPath, 'utf8'), project.localConfigPath);
  const effectiveConfigPath = localConfig.hasPackageDirs
    ? project.localConfigPath
    : project.baseConfigPath;
  const rawPackageDirs = localConfig.hasPackageDirs
    ? localConfig.packageDirs
    : baseConfig.packageDirs;
  return {
    configPath: effectiveConfigPath,
    configPaths: [
      ...(project.baseConfigPath === undefined ? [] : [project.baseConfigPath]),
      project.localConfigPath ?? join(project.projectRoot, localProjectConfigFile),
    ],
    packageDirs: uniquePaths(rawPackageDirs.map((path) => resolve(project.projectRoot, path))),
    projectRoot: project.projectRoot,
  };
}

export async function resolvePackageDirsForCommand({
  startPath = process.cwd(),
  cliPackageDirs = [],
  cwd = process.cwd(),
} = {}) {
  if (cliPackageDirs.length > 0) {
    return uniquePaths(cliPackageDirs.map((path) => resolve(cwd, path)));
  }
  return (await readProjectPackageDirs(startPath)).packageDirs;
}

export async function findProjectConfig(startPath = process.cwd()) {
  let current = await projectSearchStart(startPath);
  while (true) {
    const baseConfigPath = join(current, projectConfigFile);
    const localConfigPath = join(current, localProjectConfigFile);
    const hasBaseConfig = await isFile(baseConfigPath);
    const hasLocalConfig = await isFile(localConfigPath);
    if (hasBaseConfig || hasLocalConfig) {
      return {
        baseConfigPath: hasBaseConfig ? baseConfigPath : undefined,
        localConfigPath: hasLocalConfig ? localConfigPath : undefined,
        projectRoot: current,
      };
    }
    const parent = dirname(current);
    if (parent === current) {
      return undefined;
    }
    current = parent;
  }
}

async function projectSearchStart(startPath) {
  const absolutePath = resolve(startPath);
  try {
    const metadata = await stat(absolutePath);
    return metadata.isDirectory() ? absolutePath : dirname(absolutePath);
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return dirname(absolutePath);
    }
    throw error;
  }
}

async function isFile(path) {
  try {
    return (await stat(path)).isFile();
  } catch (error) {
    if (error?.code === 'ENOENT' || error?.code === 'ENOTDIR') {
      return false;
    }
    throw error;
  }
}

function parseProjectConfig(text, path) {
  let packageDirs = [];
  let hasPackageDirs = false;
  let readingPackageDirs = false;
  let packageDirsIndent = 0;

  for (const [lineIndex, rawLine] of text.split(/\r?\n/).entries()) {
    const line = rawLine.replace(/\r$/, '');
    const trimmed = line.trim();
    if (trimmed.length === 0 || trimmed.startsWith('#') || trimmed === '---') {
      continue;
    }
    if (line.startsWith('\t')) {
      throw new Error(`${path}:${lineIndex + 1}: tabs are not supported in skiff.yml indentation`);
    }

    const indent = line.length - line.trimStart().length;
    const topLevelMatch = indent === 0
      ? /^([A-Za-z_][A-Za-z0-9_-]*)\s*:\s*(.*)$/.exec(line)
      : null;
    if (topLevelMatch) {
      const [, key, rawValue] = topLevelMatch;
      readingPackageDirs = false;
      if (key !== 'packageDirs') {
        continue;
      }
      if (yamlStringScalarHasContent(rawValue)) {
        throw new Error(`${path}:${lineIndex + 1}: packageDirs must be a block list`);
      }
      packageDirs = [];
      hasPackageDirs = true;
      readingPackageDirs = true;
      packageDirsIndent = indent;
      continue;
    }

    if (!readingPackageDirs) {
      continue;
    }
    if (indent <= packageDirsIndent) {
      readingPackageDirs = false;
      continue;
    }
    const itemMatch = /^\s*-\s*(.*)$/.exec(line);
    if (!itemMatch) {
      throw new Error(`${path}:${lineIndex + 1}: packageDirs entries must use "- <path>"`);
    }
    const value = parseYamlStringScalar(itemMatch[1]);
    if (value.length === 0) {
      throw new Error(`${path}:${lineIndex + 1}: packageDirs entry must be a non-empty string`);
    }
    packageDirs.push(value);
  }

  return { hasPackageDirs, packageDirs };
}

function uniquePaths(paths) {
  return [...new Set(paths.map((path) => resolve(path)))];
}
