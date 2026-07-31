macro_rules! impl_raw_write_api {
    () => {
        fn insert_many_result<'a>(
            &'a self,
            _type_name: &'a str,
            _values: Vec<DbDocument>,
        ) -> DbCapabilityFuture<'a, DbWriteResult> {
            Box::pin(async { Ok(DbWriteResult::new(json!({ "inserted": 0 }))) })
        }

        fn update_one<'a>(
            &'a self,
            _type_name: &'a str,
            _selector: DbOneSelector,
            _change: ServiceDbChange,
        ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
            Box::pin(async { Ok(None) })
        }

        fn update_one_runtime<'a>(
            &'a self,
            _type_name: &'a str,
            _selector: DbOneSelector,
            _change: DbRuntimeChange,
            _heap: &'a mut RequestHeap,
            _context: DbRecoverableRuntimeContext,
        ) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
            self.state.record_legacy_runtime_call();
            Box::pin(async { Err(DbCapabilityError::decode("legacy runtime path called")) })
        }

        fn update_many<'a>(
            &'a self,
            _type_name: &'a str,
            _query: DbQuery,
            _change: ServiceDbChange,
        ) -> DbCapabilityFuture<'a, DbWriteResult> {
            Box::pin(async { Ok(DbWriteResult::new(json!({ "updated": 0 }))) })
        }

        fn upsert_by_key<'a>(
            &'a self,
            _type_name: &'a str,
            _key: DbKey,
            _insert: DbDocument,
            _change: ServiceDbChange,
        ) -> DbCapabilityFuture<'a, DbWriteResult> {
            Box::pin(async { Ok(DbWriteResult::new(json!({ "upserted": 0 }))) })
        }

        fn replace_one<'a>(
            &'a self,
            _type_name: &'a str,
            _selector: DbOneSelector,
            _value: DbDocument,
        ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
            Box::pin(async { Ok(None) })
        }

        fn replace_one_runtime<'a>(
            &'a self,
            _type_name: &'a str,
            _selector: DbOneSelector,
            _value: &'a RuntimeValue,
            _heap: &'a mut RequestHeap,
            _context: DbRecoverableRuntimeContext,
        ) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
            self.state.record_legacy_runtime_call();
            Box::pin(async { Err(DbCapabilityError::decode("legacy runtime path called")) })
        }

        fn delete_one<'a>(
            &'a self,
            _type_name: &'a str,
            _selector: DbOneSelector,
        ) -> DbCapabilityFuture<'a, bool> {
            Box::pin(async { Ok(false) })
        }

        fn delete_many<'a>(
            &'a self,
            _type_name: &'a str,
            _query: DbQuery,
        ) -> DbCapabilityFuture<'a, DbWriteResult> {
            Box::pin(async { Ok(DbWriteResult::new(json!({ "deleted": 0 }))) })
        }
    };
}

pub(super) use impl_raw_write_api;
