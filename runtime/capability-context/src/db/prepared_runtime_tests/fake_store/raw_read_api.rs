macro_rules! impl_raw_read_api {
    () => {
        fn begin_transaction(&self) -> DbCapabilityFuture<'_, ()> {
            self.state.record_raw_call();
            Box::pin(async { Ok(()) })
        }

        fn commit_transaction(&self) -> DbCapabilityFuture<'_, ()> {
            self.state.record_raw_call();
            Box::pin(async { Ok(()) })
        }

        fn abort_transaction(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            self.state.record_raw_call();
            Box::pin(async {})
        }

        fn find_one_by_key<'a>(
            &'a self,
            _type_name: &'a str,
            _key: DbKey,
            _projection: Option<Vec<FieldPath>>,
        ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
            self.state.record_raw_call();
            Box::pin(async { Ok(Some(DbDocument::new(json!({ "id": "raw-1" })))) })
        }

        fn find_one_by_key_runtime<'a>(
            &'a self,
            _type_name: &'a str,
            _key: DbKey,
            _projection: Option<Vec<FieldPath>>,
            _heap: &'a mut RequestHeap,
            _context: DbRecoverableRuntimeContext,
        ) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
            self.state.record_legacy_runtime_call();
            Box::pin(async { Err(DbCapabilityError::decode("legacy runtime path called")) })
        }

        fn find_one_by_query<'a>(
            &'a self,
            _type_name: &'a str,
            _query: DbQuery,
            _order: Vec<DbOrderEntry>,
            _projection: Option<Vec<FieldPath>>,
        ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
            Box::pin(async { Ok(None) })
        }

        fn find_one_by_query_runtime<'a>(
            &'a self,
            _type_name: &'a str,
            _query: DbQuery,
            _order: Vec<DbOrderEntry>,
            _projection: Option<Vec<FieldPath>>,
            _heap: &'a mut RequestHeap,
            _context: DbRecoverableRuntimeContext,
        ) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
            self.state.record_legacy_runtime_call();
            Box::pin(async { Err(DbCapabilityError::decode("legacy runtime path called")) })
        }

        fn find_many_page<'a>(
            &'a self,
            _type_name: &'a str,
            _query: DbQuery,
            _options: ServiceDbFindOptions,
            _projection: Option<Vec<FieldPath>>,
        ) -> DbCapabilityFuture<'a, DbPageResult> {
            Box::pin(async { Ok(DbPageResult { values: Vec::new() }) })
        }

        fn find_many_page_runtime<'a>(
            &'a self,
            _type_name: &'a str,
            _query: DbQuery,
            _options: ServiceDbFindOptions,
            _projection: Option<Vec<FieldPath>>,
            _heap: &'a mut RequestHeap,
            _context: DbRecoverableRuntimeContext,
        ) -> DbCapabilityFuture<'a, Vec<RuntimeValue>> {
            self.state.record_legacy_runtime_call();
            Box::pin(async { Err(DbCapabilityError::decode("legacy runtime path called")) })
        }

        fn create<'a>(
            &'a self,
            _type_name: &'a str,
            value: DbDocument,
        ) -> DbCapabilityFuture<'a, DbDocument> {
            Box::pin(async move { Ok(value) })
        }

        fn create_runtime<'a>(
            &'a self,
            _type_name: &'a str,
            _value: &'a RuntimeValue,
            _heap: &'a RequestHeap,
            _context: DbRecoverableRuntimeContext,
        ) -> DbCapabilityFuture<'a, RuntimeValue> {
            self.state.record_legacy_runtime_call();
            Box::pin(async { Err(DbCapabilityError::decode("legacy runtime path called")) })
        }

        fn count<'a>(
            &'a self,
            _type_name: &'a str,
            _query: DbQuery,
        ) -> DbCapabilityFuture<'a, u64> {
            Box::pin(async { Ok(0) })
        }

        fn exists_by_key<'a>(
            &'a self,
            _type_name: &'a str,
            _key: DbKey,
        ) -> DbCapabilityFuture<'a, bool> {
            Box::pin(async { Ok(false) })
        }

        fn exists_by_query<'a>(
            &'a self,
            _type_name: &'a str,
            _query: DbQuery,
        ) -> DbCapabilityFuture<'a, bool> {
            Box::pin(async { Ok(false) })
        }

        fn claim_lease<'a>(
            &'a self,
            _type_name: &'a str,
            _key: DbKey,
            _slot: &'a str,
        ) -> DbCapabilityFuture<'a, Option<DbCapabilityLeaseHandle>> {
            self.state.record_raw_call();
            Box::pin(async {
                Ok(Some(DbCapabilityLeaseHandle::new(
                    test_hold(),
                    DbDocument::new(json!({ "lease": "value" })),
                    1_000,
                )))
            })
        }

        fn renew_lease<'a>(
            &'a self,
            _hold: &'a DbCapabilityLeaseHold,
        ) -> DbCapabilityFuture<'a, bool> {
            Box::pin(async { Ok(true) })
        }

        fn release_lease<'a>(
            &'a self,
            _hold: &'a DbCapabilityLeaseHold,
        ) -> DbCapabilityFuture<'a, ()> {
            self.state.record_raw_call();
            Box::pin(async { Ok(()) })
        }

        fn read_lease<'a>(
            &'a self,
            _type_name: &'a str,
            _key: DbKey,
            _slot: &'a str,
        ) -> DbCapabilityFuture<'a, Option<Value>> {
            self.state.record_raw_call();
            Box::pin(async { Ok(Some(json!({ "lease": "value" }))) })
        }

        fn lease_lost(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
            Box::pin(async { false })
        }

        fn insert_skiff_file_record<'a>(
            &'a self,
            _record: FileCapabilityRecord,
        ) -> DbCapabilityFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }

        fn find_skiff_file_by_id<'a>(
            &'a self,
            _id: &'a str,
        ) -> DbCapabilityFuture<'a, Option<FileCapabilityRecord>> {
            Box::pin(async { Ok(None) })
        }

        fn delete_skiff_file_by_id<'a>(
            &'a self,
            _id: &'a str,
        ) -> DbCapabilityFuture<'a, u64> {
            Box::pin(async { Ok(0) })
        }
    };
}

pub(super) use impl_raw_read_api;
