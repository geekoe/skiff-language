import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  MONGOSH_EJSON_MARKER,
  createMongoshCommand,
} from '../lib/mongosh-json-command.mjs';
import { createPackageLiveCommand } from '../lib/package-live-command.mjs';

test('package-live command trims success streams and parses JSON through injected runner', async () => {
  const calls = [];
  const checkedRunner = async (...input) => {
    calls.push(input);
    return { stdout: '  {"ok":true}\n', stderr: ' warning \n' };
  };
  const command = createPackageLiveCommand({
    skiffCli: '/checkout/scripts/skiff.mjs',
    cwd: '/checkout',
    env: { PATH: '/fake' },
    nodeCommand: '/node',
    checkedRunner,
  });

  assert.deepEqual(await command.runCli(['package', 'remote', 'ping']), {
    stdout: '{"ok":true}',
    stderr: 'warning',
  });
  assert.deepEqual(await command.runCliJson(['package', 'resolve', 'safe-ref']), {
    ok: true,
  });
  assert.deepEqual(calls[0], [
    '/node',
    ['/checkout/scripts/skiff.mjs', 'package', 'remote', 'ping'],
    { cwd: '/checkout', env: { PATH: '/fake' } },
  ]);
});

test('package-live invalid JSON keeps explicit stdout diagnostics without retaining argv', async () => {
  const secret = 'package-live-secret-argv';
  const command = createPackageLiveCommand({
    skiffCli: '/checkout/scripts/skiff.mjs',
    checkedRunner: async () => ({ stdout: 'not-json', stderr: '' }),
  });
  await assert.rejects(
    command.runCliJson(['package', 'publish', secret]),
    (error) => {
      assert.match(error.message, /skiff package publish returned invalid JSON/);
      assert.match(error.message, /not-json/);
      assert.doesNotMatch(error.message, new RegExp(secret));
      assert.equal(Object.hasOwn(error, 'cause'), false);
      return true;
    },
  );
});

test('package-live rebuilds nonzero, signal, and spawn failures from explicit safe fields', async () => {
  const cases = [
    { code: 17, signal: null, expected: /exited with 17/ },
    { code: null, signal: 'SIGTERM', expected: /exited with SIGTERM/ },
    { code: 'ENOENT', signal: null, expected: /exited with ENOENT/ },
  ];
  for (const fixture of cases) {
    const secret = `package-arg-${fixture.code ?? fixture.signal}`;
    const command = createPackageLiveCommand({
      skiffCli: '/checkout/scripts/skiff.mjs',
      checkedRunner: async () => {
        throw checkedFailure({
          code: fixture.code,
          signal: fixture.signal,
          stdout: 'domain stdout',
          stderr: 'domain stderr',
          secret,
        });
      },
    });
    await assert.rejects(
      command.runCli(['package', 'pull', secret]),
      (error) => {
        assert.match(error.message, fixture.expected);
        assert.match(error.message, /stderr:\ndomain stderr/);
        assert.match(error.message, /stdout:\ndomain stdout/);
        assert.doesNotMatch(error.message, new RegExp(secret));
        assert.equal(Object.hasOwn(error, 'cause'), false);
        assert.equal(Object.hasOwn(error, 'args'), false);
        return true;
      },
    );
  }
});

test('mongosh JSON adapter parses its marker and keeps command arguments inside the runner', async () => {
  const calls = [];
  const mongosh = createMongoshCommand({
    checkedRunner: async (...input) => {
      calls.push(input);
      return {
        stdout: `startup\n${MONGOSH_EJSON_MARKER}{"value":{"$numberInt":"3"}}\n`,
        stderr: '',
      };
    },
  });
  assert.deepEqual(await mongosh.json({
    url: 'mongodb://127.0.0.1:27017/test',
    expression: 'db.values.findOne()',
    cwd: '/checkout',
  }), { value: { $numberInt: '3' } });
  assert.equal(calls[0][0], 'mongosh');
  assert.ok(calls[0][1].includes('--eval'));
  assert.match(calls[0][1].at(-1), /db\.values\.findOne/);
  assert.deepEqual(calls[0][2], { cwd: '/checkout' });
});

test('mongosh marker absence and invalid JSON preserve explicit success diagnostics', async () => {
  const missing = createMongoshCommand({
    checkedRunner: async () => ({ stdout: 'no marker', stderr: 'marker warning' }),
  });
  await assert.rejects(
    missing.json({ url: 'mongodb://test', expression: '1', cwd: '/checkout' }),
    (error) => {
      assert.match(error.message, /did not contain EJSON marker/);
      assert.match(error.message, /stdout:\nno marker/);
      assert.match(error.message, /stderr:\nmarker warning/);
      return true;
    },
  );

  const invalid = createMongoshCommand({
    checkedRunner: async () => ({
      stdout: `${MONGOSH_EJSON_MARKER}{invalid}`,
      stderr: '',
    }),
  });
  await assert.rejects(
    invalid.json({ url: 'mongodb://test', expression: '1', cwd: '/checkout' }),
    SyntaxError,
  );
});

test('mongosh rebuilds failures with both streams and never repeats eval argv', async () => {
  const expressionSecret = 'eval-expression-secret';
  const rawSecret = 'raw-error-secret';
  const cases = [
    { code: 41, signal: null, expected: /exited with 41/ },
    { code: null, signal: 'SIGKILL', expected: /exited with SIGKILL/ },
    { code: 'ENOENT', signal: null, expected: /exited with ENOENT/ },
  ];
  for (const fixture of cases) {
    const mongosh = createMongoshCommand({
      checkedRunner: async () => {
        throw checkedFailure({
          code: fixture.code,
          signal: fixture.signal,
          stdout: 'mongosh stdout',
          stderr: 'mongosh stderr',
          secret: rawSecret,
        });
      },
    });
    await assert.rejects(
      mongosh.json({
        url: 'mongodb://test',
        expression: expressionSecret,
        cwd: '/checkout',
      }),
      (error) => {
        assert.match(error.message, fixture.expected);
        assert.match(error.message, /stderr:\nmongosh stderr/);
        assert.match(error.message, /stdout:\nmongosh stdout/);
        assert.doesNotMatch(error.message, new RegExp(expressionSecret));
        assert.doesNotMatch(error.message, /--eval/);
        assert.doesNotMatch(error.message, new RegExp(rawSecret));
        assert.equal(Object.hasOwn(error, 'cause'), false);
        assert.equal(Object.hasOwn(error, 'args'), false);
        return true;
      },
    );
  }
});

function checkedFailure({ code, signal, stdout, stderr, secret }) {
  const error = new Error(`raw failure ${secret}`);
  error.code = code;
  error.signal = signal;
  error.args = ['--eval', secret];
  error.cause = new Error(secret);
  Object.defineProperties(error, {
    stdout: { value: stdout, enumerable: false },
    stderr: { value: stderr, enumerable: false },
  });
  return error;
}
