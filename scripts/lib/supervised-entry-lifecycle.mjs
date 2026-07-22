import { once } from 'node:events';

export function createSupervisedEntryLifecycle({
  component,
  child,
  pgid,
  stdoutHandle,
  stderrHandle,
  stopProcessGroup,
  isProcessGroupAlive,
  removePidMetadata,
}) {
  let primaryError;
  let primaryRecorded = false;
  let completionStarted = false;
  let releaseCompletion;

  const exit = child.exitCode !== null || child.signalCode !== null
    ? Promise.resolve({ code: child.exitCode, signal: child.signalCode })
    : once(child, 'exit').then(([code, signal]) => ({ code, signal }));
  const completionTrigger = new Promise((resolve) => {
    releaseCompletion = resolve;
  });

  let closeLogsPromise;
  const closeLogsOnce = () => {
    closeLogsPromise ??= settleCleanup([
      ['stdout-close', () => stdoutHandle.close(), 'stdout'],
      ['stderr-close', () => stderrHandle.close(), 'stderr'],
    ]).then(({ errors }) => errors);
    return closeLogsPromise;
  };

  const completion = completionTrigger.then(async () => {
    const cleanupErrors = [];
    const groupCheck = await settleCleanup([
      ['process-group-check', () => isProcessGroupAlive(pgid)],
    ]);
    cleanupErrors.push(...groupCheck.errors);
    if (groupCheck.values[0] === true) {
      cleanupErrors.push(...(await settleCleanup([
        ['process-group-stop', () => stopProcessGroup(pgid)],
      ])).errors);
    }
    cleanupErrors.push(...(await settleCleanup([
      ['child-exit', () => exit],
    ])).errors);
    cleanupErrors.push(...await closeLogsOnce());
    cleanupErrors.push(...(await settleCleanup([
      ['pid-metadata-remove', () => removePidMetadata()],
    ])).errors);

    const errors = primaryRecorded ? [primaryError, ...cleanupErrors] : cleanupErrors;
    if (errors.length === 0) {
      return await exit;
    }
    if (errors.length === 1 && primaryRecorded) {
      throw primaryError;
    }
    throw new AggregateError(
      errors,
      `[skiff-instance] ${component} lifecycle cleanup failed`,
      { cause: errors },
    );
  });

  // A supervised child may exit while its caller is still completing startup.
  // Keep the canonical promise handled without changing what its awaiters observe.
  void completion.catch(() => {});

  const beginCompletion = () => {
    if (!completionStarted) {
      completionStarted = true;
      releaseCompletion();
    }
    return completion;
  };

  const recordPrimary = (error) => {
    if (!primaryRecorded) {
      primaryRecorded = true;
      primaryError = error;
    }
  };
  const finish = () => beginCompletion();
  const stop = (_reason) => {
    return beginCompletion();
  };

  void exit.then(
    () => {
      finish();
    },
    (error) => {
      recordPrimary(error);
      finish();
    },
  );

  return { exit, completion, recordPrimary, finish, stop };

  async function settleCleanup(steps) {
    const results = await Promise.allSettled(
      steps.map(([, operation]) => Promise.resolve().then(operation)),
    );
    const values = [];
    const errors = [];
    for (let index = 0; index < results.length; index += 1) {
      const result = results[index];
      if (result.status === 'fulfilled') {
        values[index] = result.value;
      } else {
        const [step, , stream] = steps[index];
        errors.push(contextualCleanupError(result.reason, { component, step, stream }));
      }
    }
    return { errors, values };
  }
}

function contextualCleanupError(cause, { component, step, stream }) {
  const streamContext = stream === undefined ? '' : ` stream=${stream}`;
  const error = new Error(
    `[skiff-instance] ${component} cleanup failed at ${step}${streamContext}: ${errorMessage(cause)}`,
    { cause },
  );
  error.component = component;
  error.step = step;
  if (stream !== undefined) {
    error.stream = stream;
  }
  return error;
}

function errorMessage(error) {
  return error?.message || String(error);
}
