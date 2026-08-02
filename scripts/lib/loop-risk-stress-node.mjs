import {
  execFile as execLoopRiskCpuSample,
  execFile as execLoopRiskPgrep,
} from 'node:child_process';
import fs from 'node:fs/promises';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { pollLoopRiskHealth } from './loop-risk-health.mjs';

export async function loadRouterWebSocket(cliUrl) {
  const scriptDir = dirname(fileURLToPath(cliUrl));
  const scriptsRequire = createRequire(join(scriptDir, 'package.json'));
  const resolved = scriptsRequire.resolve('ws');
  const imported = await import(pathToFileURL(resolved).href);
  return imported.default ?? imported.WebSocket ?? imported;
}

export function createNodeLoopRiskStressAdapters(WebSocket, {
  onCpuSample = (sample) => {
    console.log(JSON.stringify({ event: 'runtime_cpu_sample', ...sample }));
  },
} = {}) {
  return {
    createWebSocket: (url, options) => new WebSocket(url, options),
    isWebSocketOpen: (socket) => socket.readyState === WebSocket.OPEN,
    isPidAlive(pid) {
      try {
        process.kill(pid, 0);
        return true;
      } catch {
        return false;
      }
    },
    async readCpu(pid) {
      const { stdout } = await readProcessCpu(pid);
      const value = Number(stdout.trim());
      if (!Number.isFinite(value) || stdout.trim().length === 0) {
        throw new Error(`ps returned no valid CPU sample for runtime PID ${pid}`);
      }
      return value;
    },
    readLog: (file) => fs.readFile(file, 'utf8'),
    now: Date.now,
    sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
    setTimer: setTimeout,
    clearTimer: clearTimeout,
    pollHealth: (input) => pollLoopRiskHealth(input),
    onCpuSample,
  };
}

export async function resolveRuntimePidsFromPgrep(explicitPids, runtimePgrep) {
  if (explicitPids.length > 0) {
    return explicitPids;
  }
  try {
    const { stdout } = await findRuntimePids(runtimePgrep);
    const pids = stdout
      .split(/\s+/)
      .filter(Boolean)
      .map((value) => Number(value));
    if (pids.some((pid) => !Number.isInteger(pid) || pid <= 0)) {
      throw new Error('pgrep returned an invalid runtime PID');
    }
    const selected = Array.from(new Set(pids.filter((pid) => pid !== process.pid)));
    if (selected.length > 0) {
      return selected;
    }
  } catch (error) {
    if (error?.code === 'ENOENT') {
      throw new Error('explicit --runtime-pgrep requires pgrep on PATH');
    }
  }
  throw new Error('no runtime pid found for explicit --runtime-pgrep');
}

function readProcessCpu(pid) {
  return new Promise((resolvePromise, reject) => {
    // child-process-owner: loop-risk-cpu-sample
    execLoopRiskCpuSample(
      'ps',
      ['-o', '%cpu=', '-p', String(pid)],
      { encoding: 'utf8' },
      (error, stdout, stderr) => {
        if (error) {
          reject(error);
          return;
        }
        resolvePromise({ stdout, stderr });
      },
    );
  });
}

function findRuntimePids(pattern) {
  return new Promise((resolvePromise, reject) => {
    // child-process-owner: loop-risk-pgrep
    execLoopRiskPgrep(
      'pgrep',
      ['-f', pattern],
      { encoding: 'utf8' },
      (error, stdout, stderr) => {
        if (error) {
          reject(error);
          return;
        }
        resolvePromise({ stdout, stderr });
      },
    );
  });
}
