use futures_util::StreamExt;
use mongodb::{
    bson::Document,
    error::{
        Error as MongoError, ErrorKind as MongoErrorKind, WriteFailure, TRANSIENT_TRANSACTION_ERROR,
    },
    options::ReturnDocument,
    results::{DeleteResult, InsertManyResult, UpdateResult},
    ClientSession, Collection,
};

use crate::{DbConstraintTarget, Result, ServiceDbError};

pub struct MongoSessionExecutor<'a> {
    pub collection: Collection<Document>,
    session: Option<&'a mut ClientSession>,
    constraint_context: Option<MongoConstraintContext>,
}

#[derive(Clone, Debug)]
pub struct MongoConstraintContext {
    pub target: DbConstraintTarget,
}

#[derive(Clone, Debug, Default)]
pub struct MongoFindOnePlan {
    pub filter: Document,
    pub sort: Option<Document>,
    pub projection: Option<Document>,
}

#[derive(Clone, Debug, Default)]
pub struct MongoFindManyPlan {
    pub filter: Document,
    pub sort: Option<Document>,
    pub projection: Option<Document>,
    pub limit: Option<i64>,
    pub offset: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct MongoOneWritePlan {
    pub filter: Document,
    pub sort: Option<Document>,
}

impl<'a> MongoSessionExecutor<'a> {
    pub fn new(
        collection: Collection<Document>,
        session: Option<&'a mut ClientSession>,
        constraint_context: Option<MongoConstraintContext>,
    ) -> Self {
        Self {
            collection,
            session,
            constraint_context,
        }
    }

    pub fn has_session(&self) -> bool {
        self.session.is_some()
    }

    pub fn session_mut(&mut self) -> Option<&mut ClientSession> {
        self.session.as_deref_mut()
    }

    pub async fn find_one(&mut self, plan: MongoFindOnePlan) -> Result<Option<Document>> {
        let constraint_context = self.constraint_context.clone();
        let mut action = self.collection.find_one(plan.filter);
        if let Some(sort) = plan.sort {
            action = action.sort(sort);
        }
        if let Some(projection) = plan.projection {
            action = action.projection(projection);
        }
        match &mut self.session {
            Some(session) => action
                .session(&mut **session)
                .await
                .map_err(|error| map_mongo_error(error, constraint_context.as_ref())),
            None => action
                .await
                .map_err(|error| map_mongo_error(error, constraint_context.as_ref())),
        }
    }

    pub async fn find_many(&mut self, plan: MongoFindManyPlan) -> Result<Vec<Document>> {
        let constraint_context = self.constraint_context.clone();
        let mut action = self.collection.find(plan.filter);
        if let Some(sort) = plan.sort {
            action = action.sort(sort);
        }
        if let Some(offset) = plan.offset {
            action = action.skip(offset);
        }
        if let Some(limit) = plan.limit {
            action = action.limit(limit);
        }
        if let Some(projection) = plan.projection {
            action = action.projection(projection);
        }

        let mut documents = Vec::new();
        match &mut self.session {
            Some(session) => {
                let mut cursor = action
                    .session(&mut **session)
                    .await
                    .map_err(|error| map_mongo_error(error, constraint_context.as_ref()))?;
                let mut stream = cursor.stream(&mut **session);
                while let Some(document) = stream
                    .next()
                    .await
                    .transpose()
                    .map_err(|error| map_mongo_error(error, constraint_context.as_ref()))?
                {
                    documents.push(document);
                }
            }
            None => {
                let mut cursor = action
                    .await
                    .map_err(|error| map_mongo_error(error, constraint_context.as_ref()))?;
                while let Some(document) = cursor
                    .next()
                    .await
                    .transpose()
                    .map_err(|error| map_mongo_error(error, constraint_context.as_ref()))?
                {
                    documents.push(document);
                }
            }
        }
        Ok(documents)
    }

    pub async fn find_one_and_update(
        &mut self,
        plan: MongoOneWritePlan,
        update: Document,
    ) -> Result<Option<Document>> {
        let constraint_context = self.constraint_context.clone();
        let mut action = self
            .collection
            .find_one_and_update(plan.filter, update)
            .return_document(ReturnDocument::After);
        if let Some(sort) = plan.sort {
            action = action.sort(sort);
        }
        match &mut self.session {
            Some(session) => action
                .session(&mut **session)
                .await
                .map_err(|error| map_mongo_error(error, constraint_context.as_ref())),
            None => action
                .await
                .map_err(|error| map_mongo_error(error, constraint_context.as_ref())),
        }
    }

    pub async fn find_one_and_replace(
        &mut self,
        plan: MongoOneWritePlan,
        replacement: Document,
    ) -> Result<Option<Document>> {
        let constraint_context = self.constraint_context.clone();
        let mut action = self
            .collection
            .find_one_and_replace(plan.filter, replacement)
            .return_document(ReturnDocument::After);
        if let Some(sort) = plan.sort {
            action = action.sort(sort);
        }
        match &mut self.session {
            Some(session) => action
                .session(&mut **session)
                .await
                .map_err(|error| map_mongo_error(error, constraint_context.as_ref())),
            None => action
                .await
                .map_err(|error| map_mongo_error(error, constraint_context.as_ref())),
        }
    }

    pub async fn find_one_and_delete(
        &mut self,
        plan: MongoOneWritePlan,
    ) -> Result<Option<Document>> {
        let constraint_context = self.constraint_context.clone();
        let mut action = self.collection.find_one_and_delete(plan.filter);
        if let Some(sort) = plan.sort {
            action = action.sort(sort);
        }
        match &mut self.session {
            Some(session) => action
                .session(&mut **session)
                .await
                .map_err(|error| map_mongo_error(error, constraint_context.as_ref())),
            None => action
                .await
                .map_err(|error| map_mongo_error(error, constraint_context.as_ref())),
        }
    }

    pub async fn update_many(
        &mut self,
        filter: Document,
        update: Document,
    ) -> Result<UpdateResult> {
        let constraint_context = self.constraint_context.clone();
        let action = self.collection.update_many(filter, update);
        match &mut self.session {
            Some(session) => action
                .session(&mut **session)
                .await
                .map_err(|error| map_mongo_error(error, constraint_context.as_ref())),
            None => action
                .await
                .map_err(|error| map_mongo_error(error, constraint_context.as_ref())),
        }
    }

    pub async fn update_one_upsert(
        &mut self,
        filter: Document,
        update: Document,
    ) -> Result<UpdateResult> {
        let constraint_context = self.constraint_context.clone();
        let action = self.collection.update_one(filter, update).upsert(true);
        match &mut self.session {
            Some(session) => action
                .session(&mut **session)
                .await
                .map_err(|error| map_mongo_error(error, constraint_context.as_ref())),
            None => action
                .await
                .map_err(|error| map_mongo_error(error, constraint_context.as_ref())),
        }
    }

    pub async fn delete_many(&mut self, filter: Document) -> Result<DeleteResult> {
        let constraint_context = self.constraint_context.clone();
        let action = self.collection.delete_many(filter);
        match &mut self.session {
            Some(session) => action
                .session(&mut **session)
                .await
                .map_err(|error| map_mongo_error(error, constraint_context.as_ref())),
            None => action
                .await
                .map_err(|error| map_mongo_error(error, constraint_context.as_ref())),
        }
    }

    pub async fn insert_one(&mut self, document: Document) -> Result<()> {
        let constraint_context = self.constraint_context.clone();
        let action = self.collection.insert_one(document);
        match &mut self.session {
            Some(session) => {
                action
                    .session(&mut **session)
                    .await
                    .map_err(|error| map_mongo_error(error, constraint_context.as_ref()))?;
            }
            None => {
                action
                    .await
                    .map_err(|error| map_mongo_error(error, constraint_context.as_ref()))?;
            }
        }
        Ok(())
    }

    pub async fn insert_many(&mut self, documents: Vec<Document>) -> Result<InsertManyResult> {
        let constraint_context = self.constraint_context.clone();
        let action = self.collection.insert_many(documents);
        match &mut self.session {
            Some(session) => action
                .session(&mut **session)
                .await
                .map_err(|error| map_mongo_error(error, constraint_context.as_ref())),
            None => action
                .await
                .map_err(|error| map_mongo_error(error, constraint_context.as_ref())),
        }
    }

    pub async fn count_documents(&mut self, filter: Document) -> Result<u64> {
        let constraint_context = self.constraint_context.clone();
        let action = self.collection.count_documents(filter);
        match &mut self.session {
            Some(session) => action
                .session(&mut **session)
                .await
                .map_err(|error| map_mongo_error(error, constraint_context.as_ref())),
            None => action
                .await
                .map_err(|error| map_mongo_error(error, constraint_context.as_ref())),
        }
    }
}

fn map_mongo_error(
    error: MongoError,
    constraint_context: Option<&MongoConstraintContext>,
) -> ServiceDbError {
    match constraint_context {
        Some(context) => ServiceDbError::Mongo(error).classify_write_constraint(&context.target),
        None => ServiceDbError::Mongo(error),
    }
}

const MONGO_DUPLICATE_KEY_CODE: i32 = 11000;
const MONGO_NAMESPACE_NOT_FOUND_CODE: i32 = 26;
const MONGO_WRITE_CONFLICT_CODE: i32 = 112;

pub fn is_mongo_duplicate_key_error(error: &MongoError) -> bool {
    mongo_error_has_code(error, is_mongo_duplicate_key_code)
}

pub fn is_mongo_namespace_not_found_error(error: &MongoError) -> bool {
    mongo_error_has_code(error, |code| code == MONGO_NAMESPACE_NOT_FOUND_CODE)
}

pub fn is_mongo_write_conflict_error(error: &MongoError) -> bool {
    mongo_error_has_code(error, is_mongo_write_conflict_code)
}

pub fn is_mongo_db_conflict_error(error: &MongoError) -> bool {
    mongo_db_conflict_markers_match(is_mongo_write_conflict_error(error), |label| {
        error.contains_label(label)
    })
}

pub fn mongo_db_conflict_markers_match(
    write_conflict_code: bool,
    contains_label: impl Fn(&str) -> bool,
) -> bool {
    write_conflict_code || contains_label(TRANSIENT_TRANSACTION_ERROR)
}

fn mongo_error_has_code(error: &MongoError, matches_code: impl Fn(i32) -> bool) -> bool {
    match error.kind.as_ref() {
        MongoErrorKind::Command(command_error) => matches_code(command_error.code),
        MongoErrorKind::Write(WriteFailure::WriteError(write_error)) => {
            matches_code(write_error.code)
        }
        MongoErrorKind::Write(WriteFailure::WriteConcernError(write_concern_error)) => {
            matches_code(write_concern_error.code)
        }
        MongoErrorKind::InsertMany(insert_many_error) => {
            insert_many_error
                .write_errors
                .as_ref()
                .is_some_and(|write_errors| {
                    write_errors
                        .iter()
                        .any(|write_error| matches_code(write_error.code))
                })
                || insert_many_error
                    .write_concern_error
                    .as_ref()
                    .is_some_and(|write_concern_error| matches_code(write_concern_error.code))
        }
        MongoErrorKind::BulkWrite(bulk_write_error) => {
            bulk_write_error
                .write_errors
                .values()
                .any(|write_error| matches_code(write_error.code))
                || bulk_write_error
                    .write_concern_errors
                    .iter()
                    .any(|write_concern_error| matches_code(write_concern_error.code))
        }
        _ => false,
    }
}

pub fn is_mongo_duplicate_key_code(code: i32) -> bool {
    code == MONGO_DUPLICATE_KEY_CODE
}

pub fn is_mongo_write_conflict_code(code: i32) -> bool {
    code == MONGO_WRITE_CONFLICT_CODE
}

pub fn update_without_set_on_insert(update: &Document) -> Option<Document> {
    let mut update = update.clone();
    update.remove("$setOnInsert");
    (!update.is_empty()).then_some(update)
}
