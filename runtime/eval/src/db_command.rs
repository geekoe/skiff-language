use skiff_runtime_capability_context::{
    DbCapabilityTarget, DbDocument, DbKey, DbOrderEntry, DbQuery, FieldPath, ServiceDbChange,
    ServiceDbFindOptions,
};
use skiff_runtime_model::{runtime_value::RuntimeValue, type_plan::RuntimeTypePlan};

use crate::capabilities::{DbRecoverableRuntimeExpectedPlans, DbRuntimeChange};

#[derive(Debug)]
pub enum DbCommand {
    FindMany(DbFindManyCommand),
    FindOne(DbFindOneCommand),
    InsertOne(DbInsertOneCommand),
    InsertMany(DbInsertManyCommand),
    UpdateOne(DbUpdateOneCommand),
    UpdateMany(DbUpdateManyCommand),
    UpsertKey(DbUpsertKeyCommand),
    ReplaceOne(DbReplaceOneCommand),
    DeleteOne(DbDeleteOneCommand),
    DeleteMany(DbQueryCommand),
    Count(DbQueryCommand),
    ExistsKey(DbExistsKeyCommand),
    ExistsQuery(DbQueryCommand),
}

#[derive(Debug)]
pub struct DbFindManyCommand {
    pub target: DbCapabilityTarget,
    pub result_plan: RuntimeTypePlan,
    pub query: DbQuery,
    pub options: ServiceDbFindOptions,
    pub projection: Option<Vec<FieldPath>>,
    pub recoverable_runtime: Option<DbRecoverableRuntimeExpectedPlans>,
}

#[derive(Debug)]
pub struct DbFindOneCommand {
    pub target: DbCapabilityTarget,
    pub result_plan: RuntimeTypePlan,
    pub selector: DbOneCommandSelector,
    pub projection: Option<Vec<FieldPath>>,
    pub recoverable_runtime: Option<DbRecoverableRuntimeExpectedPlans>,
    pub required: bool,
}

#[derive(Debug)]
pub enum DbOneCommandSelector {
    Key {
        key: DbKey,
    },
    Query {
        query: DbQuery,
        order: Vec<DbOrderEntry>,
    },
}

#[derive(Debug)]
pub struct DbInsertOneCommand {
    pub target: DbCapabilityTarget,
    pub result_plan: RuntimeTypePlan,
    pub value: DbCommandValue,
}

#[derive(Debug)]
pub struct DbInsertManyCommand {
    pub target: DbCapabilityTarget,
    pub result_plan: RuntimeTypePlan,
    pub values: Vec<DbDocument>,
}

#[derive(Debug)]
pub struct DbUpdateOneCommand {
    pub target: DbCapabilityTarget,
    pub result_plan: RuntimeTypePlan,
    pub selector: DbOneCommandSelector,
    pub change: DbCommandChange,
}

#[derive(Debug)]
pub struct DbUpdateManyCommand {
    pub target: DbCapabilityTarget,
    pub result_plan: RuntimeTypePlan,
    pub query: DbQuery,
    pub change: ServiceDbChange,
}

#[derive(Debug)]
pub struct DbUpsertKeyCommand {
    pub target: DbCapabilityTarget,
    pub result_plan: RuntimeTypePlan,
    pub key: DbKey,
    pub insert: DbDocument,
    pub change: ServiceDbChange,
}

#[derive(Debug)]
pub struct DbReplaceOneCommand {
    pub target: DbCapabilityTarget,
    pub result_plan: RuntimeTypePlan,
    pub selector: DbOneCommandSelector,
    pub value: DbCommandValue,
}

#[derive(Debug)]
pub enum DbCommandValue {
    Wire(DbDocument),
    Runtime {
        value: RuntimeValue,
        recoverable_runtime: DbRecoverableRuntimeExpectedPlans,
    },
}

#[derive(Debug)]
pub enum DbCommandChange {
    Wire(ServiceDbChange),
    Runtime {
        change: DbRuntimeChange,
        recoverable_runtime: DbRecoverableRuntimeExpectedPlans,
    },
}

#[derive(Debug)]
pub struct DbDeleteOneCommand {
    pub target: DbCapabilityTarget,
    pub selector: DbOneCommandSelector,
}

#[derive(Debug)]
pub struct DbQueryCommand {
    pub target: DbCapabilityTarget,
    pub query: DbQuery,
}

#[derive(Debug)]
pub struct DbExistsKeyCommand {
    pub target: DbCapabilityTarget,
    pub key: DbKey,
}
