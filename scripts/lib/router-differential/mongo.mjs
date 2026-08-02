// Mongo seeding/reading for the differential harness.
//
// Each side owns an independent temporary mongod and an independent
// database namespace. The same semantic EnvironmentActivationState is
// seeded into each implementation's canonical collections; Mongo
// observations are the decoded semantic state and audit counts, so the
// comparison is namespace-neutral by construction.

import { createMongoshCommand } from '../mongosh-json-command.mjs';

export function createDifferentialMongosh() {
  return createMongoshCommand();
}

export async function seedActivationState({
  mongosh,
  mongoUrl,
  database,
  collection,
  environment,
  state,
}) {
  const document = {
    _id: environment,
    revision: 0,
    state,
  };
  const script = [
    `db.getSiblingDB(${JSON.stringify(database)})`,
    `.getCollection(${JSON.stringify(collection)})`,
    `.insertOne(${JSON.stringify(document)});`,
  ].join('');
  await mongosh.run([
    mongoUrl,
    '--quiet',
    '--eval',
    script,
  ], { cwd: process.cwd() });
}

export async function readActivationState({
  mongosh,
  mongoUrl,
  database,
  collection,
  environment,
}) {
  const script = [
    `db.getSiblingDB(${JSON.stringify(database)})`,
    `.getCollection(${JSON.stringify(collection)})`,
    `.findOne({_id: ${JSON.stringify(environment)}})`,
  ].join('');
  return await mongosh.json({
    url: mongoUrl,
    expression: script,
    cwd: process.cwd(),
  });
}

export async function countAuditEntries({
  mongosh,
  mongoUrl,
  database,
  collection,
}) {
  const script = [
    `db.getSiblingDB(${JSON.stringify(database)})`,
    `.getCollection(${JSON.stringify(collection)})`,
    '.countDocuments({})',
  ].join('');
  return await mongosh.json({
    url: mongoUrl,
    expression: script,
    cwd: process.cwd(),
  });
}
