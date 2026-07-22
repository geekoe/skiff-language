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
    const processGroup = await closeProcessGroup();
    cleanupErrors.push(...processGroup.errors);
    if (processGroup.absent) {
      cleanupErrors.push(...(await settleCleanup([
        ['child-exit', () => exit],
      ])).errors);
    }
    cleanupErrors.push(...await closeLogsOnce());
    if (processGroup.absent) {
      cleanupErrors.push(...(await settleCleanup([
        ['pid-metadata-remove', () => removePidMetadata()],
      ])).errors);
    }

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
  const detach = async () => {
    const closeErrors = await closeLogsOnce();
    if (closeErrors.length > 0) {
      return await beginCompletion();
    }
    if (completionStarted) {
      const error = new Error(
        `[skiff-instance] ${component} exited before process lifecycle detach`,
      );
      recordPrimary(error);
      try {
        await completion;
      } catch (completionError) {
        throw completionError;
      }
      throw error;
    }
    try {
      child.unref();
    } catch (error) {
      recordPrimary(error);
      return await beginCompletion();
    }
    return { detached: true };
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

  return { exit, completion, recordPrimary, finish, stop, detach };

  async function closeProcessGroup() {
    const errors = [];
    const groupCheck = await settleCleanup([
      ['process-group-check', async () => {
        const alive = await isProcessGroupAlive(pgid);
        if (alive !== true && alive !== false) {
          throw new Error(`process-group check returned ${String(alive)}`);
        }
        return alive;
      }],
    ]);
    errors.push(...groupCheck.errors);
    if (groupCheck.errors.length > 0) {
      return { absent: false, errors };
    }
    if (groupCheck.values[0] === false) {
      return { absent: true, errors };
    }

    const stopResult = await settleCleanup([
      ['process-group-stop', async () => {
        const result = await stopProcessGroup(pgid);
        if (result?.stopped !== true) {
          throw new Error(
            `process-group stop did not prove absence: ${JSON.stringify(result ?? null)}`,
          );
        }
        return result;
      }],
    ]);
    errors.push(...stopResult.errors);
    const absenceCheck = await settleCleanup([
      ['process-group-absence', async () => {
        const alive = await isProcessGroupAlive(pgid);
        if (alive !== false) {
          throw new Error(`process group ${pgid} is still alive after stop`);
        }
        return true;
      }],
    ]);
    errors.push(...absenceCheck.errors);
    return { absent: absenceCheck.values[0] === true, errors };
  }

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
