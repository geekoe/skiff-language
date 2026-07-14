import { captureCheckedCommand } from './command-execution.mjs';

export const MONGOSH_EJSON_MARKER = '__SKIFF_ENCRYPTED_LIVE_EJSON__';

export function createMongoshCommand({
  checkedRunner = captureCheckedCommand,
} = {}) {
  async function run(args, options = {}) {
    try {
      return await checkedRunner('mongosh', args, options);
    } catch (error) {
      const status = error?.signal ?? error?.code ?? 'UNKNOWN';
      throw new Error([
        `mongosh exited with ${status}`,
        streamDiagnostic('stderr', error?.stderr),
        streamDiagnostic('stdout', error?.stdout),
      ].filter(Boolean).join('\n'));
    }
  }

  async function json({ url, expression, cwd }) {
    const code = [
      `const value=(${expression});`,
      `print(${JSON.stringify(MONGOSH_EJSON_MARKER)}+EJSON.stringify(value,{relaxed:false}));`,
    ].join('');
    const result = await run(
      [url, '--quiet', '--eval', code],
      { cwd },
    );
    const line = result.stdout
      .split(/\r?\n/)
      .find((candidate) => candidate.startsWith(MONGOSH_EJSON_MARKER));
    if (line === undefined) {
      throw new Error([
        'mongosh result did not contain EJSON marker',
        streamDiagnostic('stdout', result.stdout),
        streamDiagnostic('stderr', result.stderr),
      ].filter(Boolean).join('\n'));
    }
    return JSON.parse(line.slice(MONGOSH_EJSON_MARKER.length));
  }

  return Object.freeze({ run, json });
}

function streamDiagnostic(label, value) {
  return typeof value === 'string' && value.trim().length > 0
    ? `${label}:\n${value.trim()}`
    : '';
}
