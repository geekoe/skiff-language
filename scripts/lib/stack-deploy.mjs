import { mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { loadStackConfig, requireRemoteStackConfig } from './stack-config.mjs';
import { createStackShell } from './stack-shell.mjs';

const BUILD_MANIFEST_SCHEMA = 'skiff-runtime-stack-build-v1';
const COPIED_CONFIG_FILES = ['router.yml', 'runtime.yml', 'telemetry.yml'];
const BINARY_UNITS = ['router', 'runtime', 'compiler'];
const REQUIRED_BINARY_UNITS = ['router', 'runtime'];
const PM2_APP_ORDER = ['telemetry', 'router', 'runtime'];

export async function deployStack({
  configDir,
  skiffRoot,
  shell = createStackShell({ skiffRoot }),
}) {
  const stack = await loadStackConfig(configDir, { skiffRoot });
  requireRemoteStackConfig(stack, 'stack deploy');
  const manifest = await readBuildManifest(path.join(stack.paths.buildRoot, 'manifest.json'));
  const binaryUnits = BINARY_UNITS.filter((unit) => hasBinaryArtifact(manifest, unit));
  for (const unit of REQUIRED_BINARY_UNITS) {
    if (!binaryUnits.includes(unit)) {
      throw new Error(
        `${unit} is missing from ${path.join(stack.paths.buildRoot, 'manifest.json')}; run stack build first`,
      );
    }
  }

  const { host, remoteSkiff, nodeBin } = stack.config.remote;
  for (const unit of binaryUnits) {
    const artifact = manifest.units[unit].artifacts.find((item) => item.kind === 'binary');
    await assertRegularFile(path.resolve(skiffRoot, artifact.path), `${unit} binary`);
  }

  await shell.remoteRun(
    host,
    `mkdir -p ${remoteSkiff}/{artifacts,bin,config,logs,telemetry,scripts,runtime-home}`,
  );

  for (const file of COPIED_CONFIG_FILES) {
    await shell.rsync(
      path.join(configDir, file),
      `${host}:${remoteSkiff}/config/${file}`,
    );
  }

  for (const unit of binaryUnits) {
    const artifact = manifest.units[unit].artifacts.find((item) => item.kind === 'binary');
    const source = path.resolve(skiffRoot, artifact.path);
    const target = path.posix.join(remoteSkiff, 'bin', binaryUnitName(unit));
    await shell.rsync(source, `${host}:${target}`);
    await shell.remoteRun(host, `chmod +x ${target}`);
  }

  if (hasTsUnit(manifest, 'telemetry')) {
    await shell.rsync(
      path.join(skiffRoot, 'telemetry') + path.sep,
      `${host}:${remoteSkiff}/telemetry/`,
      ['--exclude', 'node_modules', '--exclude', 'dist', '--exclude', 'telemetry.yml'],
    );
    await shell.remoteRun(
      host,
      `cd ${remoteSkiff}/telemetry && PATH=${nodeBin}:$PATH pnpm install --prod=false --ignore-scripts`,
    );
  }

  const tempRoot = await mkdtemp(path.join(os.tmpdir(), 'skiff-stack-deploy-'));
  try {
    const ecosystemPath = path.join(tempRoot, 'ecosystem.config.cjs');
    await writeFile(ecosystemPath, renderEcosystemConfig({ remoteSkiff, nodeBin }));
    await shell.rsync(ecosystemPath, `${host}:${remoteSkiff}/ecosystem.config.cjs`);
  } finally {
    await rm(tempRoot, { recursive: true, force: true });
  }

  const pm2 = `PATH=${nodeBin}:$PATH pm2`;
  for (const app of PM2_APP_ORDER) {
    await shell.remoteRun(host, `cd ${remoteSkiff} && ${pm2} delete ${pm2AppName(app)} || true`);
  }
  for (const app of PM2_APP_ORDER) {
    await shell.remoteRun(
      host,
      `cd ${remoteSkiff} && ${pm2} startOrReload ecosystem.config.cjs --only ${pm2AppName(app)} --update-env`,
    );
  }
  await shell.remoteRun(host, `${pm2} save`);

  return {
    remote: {
      host,
      remoteSkiff,
    },
    configDir,
    config: COPIED_CONFIG_FILES.map((file) => `${remoteSkiff}/config/${file}`),
    binaries: binaryUnits.map((unit) => path.posix.join(remoteSkiff, 'bin', binaryUnitName(unit))),
    telemetry: hasTsUnit(manifest, 'telemetry')
      ? {
          source: path.join(skiffRoot, 'telemetry'),
          remote: `${remoteSkiff}/telemetry`,
        }
      : null,
    ecosystem: `${remoteSkiff}/ecosystem.config.cjs`,
    pm2Apps: PM2_APP_ORDER.map(pm2AppName),
    buildManifest: path.join(stack.paths.buildRoot, 'manifest.json'),
    deployed: Object.fromEntries(
      binaryUnits.map((unit) => [
        unit,
        {
          commit: manifest.units[unit].commit,
          sourceKey: manifest.units[unit].sourceKey,
        },
      ]),
    ),
  };
}

export function renderEcosystemConfig({ remoteSkiff, nodeBin }) {
  return `const NODE_BIN = '${nodeBin}';

module.exports = {
  apps: [
    {
      name: 'skiff-router',
      cwd: '${remoteSkiff}',
      script: '${remoteSkiff}/bin/skiff-router',
      args: '${remoteSkiff}/config/router.yml',
      interpreter: 'none',
      watch: false,
      autorestart: true,
      max_restarts: 5,
      restart_delay: 2000,
      env: {
        RUST_LOG: 'info',
      },
    },
    {
      name: 'skiff-telemetry',
      cwd: '${remoteSkiff}/telemetry',
      script: 'src/main.ts',
      interpreter: NODE_BIN + '/node',
      interpreter_args: '--import tsx',
      args: '--config ${remoteSkiff}/config/telemetry.yml',
      watch: false,
      autorestart: true,
      max_restarts: 5,
      restart_delay: 2000,
      env: {
        NODE_ENV: 'production',
      },
    },
    {
      name: 'skiff-runtime',
      cwd: '${remoteSkiff}',
      script: '${remoteSkiff}/bin/skiff-runtime',
      args: '${remoteSkiff}/config/runtime.yml',
      interpreter: 'none',
      watch: false,
      autorestart: true,
      max_restarts: 5,
      restart_delay: 2000,
      env: {
        RUST_LOG: 'info',
      },
    },
  ],
};
`;
}

export async function readBuildManifest(file) {
  let value;
  try {
    value = JSON.parse(await readFile(file, 'utf8'));
  } catch (error) {
    if (error?.code === 'ENOENT') {
      throw new Error(`build manifest not found at ${file}; run stack build first`);
    }
    throw error;
  }
  if (value.schemaVersion !== BUILD_MANIFEST_SCHEMA) {
    throw new Error(`${file} is not a skiff runtime stack build manifest`);
  }
  return value;
}

function hasBinaryArtifact(manifest, unit) {
  return (
    manifest.units?.[unit]?.artifacts?.some((item) => item.kind === 'binary') ?? false
  );
}

function hasTsUnit(manifest, unit) {
  return manifest.units?.[unit] !== undefined;
}

function binaryUnitName(unit) {
  switch (unit) {
    case 'router':
      return 'skiff-router';
    case 'runtime':
      return 'skiff-runtime';
    case 'compiler':
      return 'skiff-compiler';
    default:
      throw new Error(`unknown binary unit ${unit}`);
  }
}

function pm2AppName(component) {
  switch (component) {
    case 'router':
      return 'skiff-router';
    case 'telemetry':
      return 'skiff-telemetry';
    case 'runtime':
      return 'skiff-runtime';
    default:
      throw new Error(`unknown PM2 app ${component}`);
  }
}

async function assertRegularFile(file, label) {
  try {
    if (!(await stat(file)).isFile()) {
      throw new Error(`${label} must be a regular file: ${file}`);
    }
  } catch (error) {
    if (error?.code === 'ENOENT') {
      throw new Error(`${label} does not exist: ${file}`);
    }
    throw error;
  }
}
