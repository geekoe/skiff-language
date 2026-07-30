import { setTimeout as delay } from 'node:timers/promises';
import { createMongoshCommand } from './mongosh-json-command.mjs';

export function createEncryptedStorageLiveMongoProbe({
  mongoPort,
  cwd,
  command = createMongoshCommand(),
  wait = delay,
}) {
  async function mongoJson(database, expression) {
    return command.json({
      url: `mongodb://127.0.0.1:${mongoPort}/${database}?directConnection=true`,
      expression,
      cwd,
    });
  }

  async function rawDocument(database, collection, id) {
    return mongoJson(
      database,
      `db.getCollection(${JSON.stringify(collection)}).findOne({_id:${JSON.stringify(id)}})`,
    );
  }

  async function rawDocuments(database, collection) {
    return mongoJson(
      database,
      `db.getCollection(${JSON.stringify(collection)}).find({}).sort({_id:1}).toArray()`,
    );
  }

  async function collectionNames(database) {
    return mongoJson(database, 'db.getCollectionNames().sort()');
  }

  async function databaseNames() {
    return mongoJson(
      'admin',
      'db.adminCommand({listDatabases:1,nameOnly:true}).databases.map((entry)=>entry.name).sort()',
    );
  }

  async function databaseExists(database) {
    return (await databaseNames()).includes(database);
  }

  async function dropDatabase(database) {
    return mongoJson(database, 'db.dropDatabase()');
  }

  async function replaceRawDocument(database, collection, id, document) {
    const serialized = JSON.stringify(document);
    return mongoJson(
      database,
      `db.getCollection(${JSON.stringify(collection)}).replaceOne({_id:${JSON.stringify(id)}}, EJSON.parse(${JSON.stringify(serialized)}))`,
    );
  }

  async function setRawFields(database, collection, id, fields) {
    const serialized = JSON.stringify(fields);
    return mongoJson(
      database,
      `db.getCollection(${JSON.stringify(collection)}).updateOne({_id:${JSON.stringify(id)}}, {$set:EJSON.parse(${JSON.stringify(serialized)})})`,
    );
  }

  async function countNotKeyId(database, collection, field, keyId) {
    return mongoJson(
      database,
      `db.getCollection(${JSON.stringify(collection)}).countDocuments({${JSON.stringify(`${field}._skiff_encrypted.keyId`)}:{$ne:${JSON.stringify(keyId)}}})`,
    );
  }

  async function observeTransientEncryptedStorage(databasesBefore) {
    for (let attempt = 0; attempt < 3000; attempt += 1) {
      const databases = await databaseNames();
      for (const database of databases) {
        if (
          databasesBefore.has(database)
          || ['admin', 'config', 'local'].includes(database)
        ) {
          continue;
        }
        const collections = await collectionNames(database);
        for (const collection of collections) {
          const documents = await rawDocuments(database, collection);
          const fields = encryptedEnvelopeFields(documents);
          if (fields.length > 0) {
            return {
              storageServiceId: storageServiceIdFromDatabase(database),
              database,
              collection,
              fields,
              keyIds: encryptedEnvelopeKeyIds(documents, fields),
              rawSnapshot: JSON.stringify(documents),
            };
          }
        }
      }
      await wait(50);
    }
    throw new Error('did not observe transient test-runner encrypted storage');
  }

  async function initializeReplicaSet() {
    const initiate = `try { rs.status(); } catch (error) { rs.initiate({_id:'rs0',members:[{_id:0,host:'127.0.0.1:${mongoPort}'}]}); }`;
    await command.run(
      [
        `mongodb://127.0.0.1:${mongoPort}/admin?directConnection=true`,
        '--quiet',
        '--eval',
        initiate,
      ],
      { cwd },
    );
    for (let attempt = 0; attempt < 60; attempt += 1) {
      try {
        const writable = await mongoJson(
          'admin',
          'db.hello().isWritablePrimary === true',
        );
        if (writable) {
          return;
        }
      } catch {
        // Replica initialization briefly closes connections.
      }
      await wait(250);
    }
    throw new Error('managed Mongo replica set did not become PRIMARY');
  }

  return Object.freeze({
    collectionNames,
    countNotKeyId,
    databaseExists,
    databaseNames,
    dropDatabase,
    initializeReplicaSet,
    mongoJson,
    observeTransientEncryptedStorage,
    rawDocument,
    rawDocuments,
    replaceRawDocument,
    setRawFields,
  });
}

function encryptedEnvelopeFields(documents) {
  const fields = new Set();
  for (const document of documents) {
    for (const [field, value] of Object.entries(document)) {
      if (
        field !== '_id'
        && value !== null
        && typeof value === 'object'
        && value._skiff_encrypted !== undefined
      ) {
        fields.add(field);
      }
    }
  }
  return [...fields].sort();
}

function encryptedEnvelopeKeyIds(documents, fields) {
  const keyIds = new Set();
  for (const document of documents) {
    for (const field of fields) {
      const keyId = document[field]?._skiff_encrypted?.keyId;
      if (keyId !== undefined) {
        keyIds.add(keyId);
      }
    }
  }
  return [...keyIds].sort();
}

function storageServiceIdFromDatabase(database) {
  return database.replaceAll('~~', '/').replaceAll('~', '.');
}
