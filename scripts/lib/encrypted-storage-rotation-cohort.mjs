import { readFile, writeFile } from 'node:fs/promises';

export const ROTATION_PAGE_SIZE = 100;

const ROTATION_BATCH_SIZE = 20;

export function createRotationInventory(config) {
  return [
    {
      storageServiceId: config.defaultService,
      database: config.defaultDatabase,
      collection: config.defaultCollection,
      fields: ['apiKey', 'refreshToken'],
      scanPath: `${config.defaultBase}/scan`,
      rewritePath: `${config.defaultBase}/rewrite-batch`,
      service: config.defaultService,
      barrierPath: `${config.defaultBase}/barrier`,
      barrierStatusPath: `${config.defaultBase}/barrier-status`,
    },
    {
      storageServiceId: config.defaultService,
      database: config.defaultDatabase,
      collection: config.archiveCollection,
      fields: ['apiKey'],
      scanPath: `${config.defaultBase}/archive-scan`,
      rewritePath: `${config.defaultBase}/archive-rewrite-batch`,
      service: config.defaultService,
      barrierPath: `${config.defaultBase}/barrier`,
      barrierStatusPath: `${config.defaultBase}/barrier-status`,
    },
    {
      storageServiceId: config.mappedService,
      database: config.mappedDatabase,
      collection: config.mappedCollection,
      fields: ['token'],
      scanPath: `${config.mappedBase}/scan`,
      rewritePath: `${config.mappedBase}/rewrite-batch`,
      service: config.mappedService,
      barrierPath: `${config.mappedBase}/barrier`,
      barrierStatusPath: `${config.mappedBase}/barrier-status`,
    },
    {
      storageServiceId: config.mappedService,
      database: config.mappedDatabase,
      collection: config.defaultCollection,
      fields: ['apiKey'],
      scanPath: `${config.mappedBase}/service-probe-scan`,
      rewritePath: `${config.mappedBase}/service-probe-rewrite-batch`,
      service: config.mappedService,
      barrierPath: `${config.mappedBase}/barrier`,
      barrierStatusPath: `${config.mappedBase}/barrier-status`,
    },
    {
      storageServiceId: config.retiredStorage.storageServiceId,
      database: config.retiredStorage.database,
      collection: config.retiredStorage.collection,
      fields: [...config.retiredStorage.fields],
      retired: true,
    },
  ];
}

export class RotationCohort {
  static async create(input) {
    let checkpoints = new Map();
    try {
      const stored = JSON.parse(await readFile(input.checkpointPath, 'utf8'));
      checkpoints = new Map(stored);
    } catch (error) {
      if (error.code !== 'ENOENT') {
        throw error;
      }
    }
    return new RotationCohort(input, checkpoints);
  }

  constructor(input, checkpoints) {
    Object.assign(this, input);
    this.checkpoints = checkpoints;
    this.writeBarrier = false;
    this.scanHistory = new Map();
    this.batchRewriteHistory = new Map();
  }

  async beginWriteBarrier() {
    for (const endpoint of barrierEndpoints(this.expectedInventory)) {
      const result = await this.harness.request(endpoint.service, endpoint.barrierPath, {
        token: this.barrierToken,
      });
      assert(result.active === true, `failed to activate writer barrier for ${endpoint.service}`);
    }
    this.writeBarrier = true;
    this.harness.requireRetirementGate();
    await this.assertRealBarrierActive();
  }

  checkpoint(entry) {
    return this.checkpoints.get(this.checkpointKey(entry));
  }

  scanCursors(entry) {
    return this.scanHistory.get(this.checkpointKey(entry)) ?? [];
  }

  batchRewrites(entry) {
    return this.batchRewriteHistory.get(this.checkpointKey(entry)) ?? [];
  }

  assertRetirementInventory(candidate) {
    assert(this.writeBarrier, 'cohort retirement requires an active full-writer barrier');
    const expected = canonicalInventory(this.expectedInventory);
    const actual = canonicalInventory(candidate);
    assert(actual === expected, 'cohort inventory is incomplete; refusing old-key retirement');
  }

  async assertRealBarrierActive() {
    assert(this.writeBarrier, 'cohort requires an active full-writer barrier');
    for (const endpoint of barrierEndpoints(this.expectedInventory)) {
      const status = await this.harness.request(endpoint.service, endpoint.barrierStatusPath, {});
      assert(status.active === true, `writer barrier is not active for ${endpoint.service}`);
    }
  }

  async migrateCollection(entry, { crashAfterFirstBatchBeforeCheckpoint = false } = {}) {
    assert(this.writeBarrier, 'rotation migration requires an active full-writer barrier');
    if (entry.retired) {
      assert(!(await this.harness.databaseExists(entry.database)), `${entry.database} must remain retired`);
      for (const field of entry.fields) {
        const count = await this.harness.countNotKeyId(
          entry.database,
          entry.collection,
          field,
          'next-v2',
        );
        assert(numberFromEjson(count) === 0, `retired ${entry.collection}.${field} is not empty`);
      }
      await this.saveCheckpoint(entry, { lastId: '', complete: true });
      return;
    }

    let cursor = this.checkpoint(entry)?.lastId ?? '';
    assert(typeof cursor === 'string', `${entry.collection} checkpoint lastId must be a string`);
    while (true) {
      this.record(this.scanHistory, entry, cursor);
      const rows = await this.harness.request(entry.service, entry.scanPath, { lastId: cursor });
      assert(rows.length <= ROTATION_PAGE_SIZE, `${entry.collection} scan exceeded its page limit`);
      assertStringPrimaryKeyPage(entry, rows, cursor);
      if (rows.length === 0) {
        await this.saveCheckpoint(entry, { lastId: cursor, complete: true });
        return;
      }

      for (let offset = 0; offset < rows.length; offset += ROTATION_BATCH_SIZE) {
        const batch = rows.slice(offset, offset + ROTATION_BATCH_SIZE);
        await this.harness.request(
          entry.service,
          entry.rewritePath,
          { rows: batch },
          { rotationToken: this.barrierToken },
        );
        this.record(this.batchRewriteHistory, entry, {
          firstId: batch[0].id,
          lastId: batch.at(-1).id,
          rowCount: batch.length,
        });
        if (crashAfterFirstBatchBeforeCheckpoint && offset === 0) {
          throw new SimulatedCheckpointCrash();
        }
        cursor = batch.at(-1).id;
        await this.saveCheckpoint(entry, { lastId: cursor, complete: false });
      }

      if (rows.length < ROTATION_PAGE_SIZE) {
        await this.saveCheckpoint(entry, { lastId: cursor, complete: true });
        return;
      }
    }
  }

  async retireOldKey(candidateInventory, nextKeyring) {
    this.assertRetirementInventory(candidateInventory);
    await this.assertRealBarrierActive();
    for (const entry of this.expectedInventory) {
      const checkpoint = this.checkpoint(entry);
      assert(checkpoint?.complete === true, `checkpoint is incomplete for ${entry.storageServiceId}/${entry.collection}`);
      if (entry.retired) {
        assert(!(await this.harness.databaseExists(entry.database)), `${entry.database} unexpectedly returned`);
      }
      for (const field of entry.fields) {
        const count = await this.harness.countNotKeyId(
          entry.database,
          entry.collection,
          field,
          nextKeyring.activeKeyId,
        );
        assert(
          numberFromEjson(count) === 0,
          `${entry.storageServiceId}/${entry.collection}.${field} blocks old-key retirement`,
        );
      }
    }
    await this.harness.restartRuntime(nextKeyring, { retirementAuthorized: true });
  }

  record(history, entry, value) {
    const key = this.checkpointKey(entry);
    const values = history.get(key) ?? [];
    values.push(value);
    history.set(key, values);
  }

  async saveCheckpoint(entry, value) {
    this.checkpoints.set(this.checkpointKey(entry), value);
    await writeFile(this.checkpointPath, `${JSON.stringify([...this.checkpoints], null, 2)}\n`, 'utf8');
  }

  checkpointKey(entry) {
    return JSON.stringify([
      this.targetFingerprint,
      entry.storageServiceId,
      entry.collection,
    ]);
  }
}

export class SimulatedCheckpointCrash extends Error {}

export function inventoryWithMissingField(inventory) {
  return inventory.map((entry, index) => ({
    ...entry,
    fields: index === 0 ? entry.fields.slice(0, 1) : [...entry.fields],
  }));
}

export function inventoryWithMissingService(inventory, serviceId) {
  return inventory.filter((entry) => entry.storageServiceId !== serviceId);
}

export function numberFromEjson(value) {
  if (typeof value === 'number') {
    return value;
  }
  return Number(value.$numberInt ?? value.$numberLong);
}

function assertStringPrimaryKeyPage(entry, rows, cursor) {
  let previousId = cursor;
  for (const row of rows) {
    assert(typeof row.id === 'string', `${entry.collection} scan returned a non-string id`);
    assert(row.id > previousId, `${entry.collection} scan did not order string ids after ${previousId}`);
    previousId = row.id;
  }
}

function canonicalInventory(inventory) {
  return JSON.stringify(
    inventory
      .map((entry) => ({
        storageServiceId: entry.storageServiceId,
        database: entry.database,
        collection: entry.collection,
        fields: [...entry.fields].sort(),
        retired: entry.retired === true,
      }))
      .sort((left, right) => `${left.storageServiceId}/${left.collection}`.localeCompare(`${right.storageServiceId}/${right.collection}`)),
  );
}

function barrierEndpoints(inventory) {
  const endpoints = new Map();
  for (const entry of inventory) {
    if (entry.retired || entry.barrierPath === undefined) {
      continue;
    }
    endpoints.set(`${entry.service}:${entry.barrierPath}`, {
      service: entry.service,
      barrierPath: entry.barrierPath,
      barrierStatusPath: entry.barrierStatusPath,
    });
  }
  return [...endpoints.values()];
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}
