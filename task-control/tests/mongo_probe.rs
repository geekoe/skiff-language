//! Real-boundary Mongo probe for `MongoTaskStore`.
//!
//! Ignored by default. The harness sets `SKIFF_TASK_CONTROL_MONGO_URL`
//! (temporary replica set) and optionally `SKIFF_TASK_CONTROL_MONGO_DB`,
//! then runs this test with `--ignored`. The probe runs the same shared
//! contract matrix as the in-memory fake, plus an index existence check.

mod support;

#[cfg(test)]
mod tests {
    use futures_util::TryStreamExt;
    use mongodb::{options::ClientOptions, Client};

    use skiff_task_control::{
        MongoTaskStore, MongoTaskStoreOptions, TaskStore, TASK_STATE_DUE_AT_INDEX,
    };

    use super::support::{contract, TestTime};

    fn required(name: &str) -> String {
        std::env::var(name).unwrap_or_else(|_| {
            panic!("{name} must be set by the task-control Mongo probe harness")
        })
    }

    fn probe_options(database: &str) -> MongoTaskStoreOptions {
        MongoTaskStoreOptions {
            database: database.to_string(),
            ..Default::default()
        }
    }

    #[tokio::test]
    #[ignore = "requires SKIFF_TASK_CONTROL_MONGO_URL temporary replica set managed by the probe harness"]
    async fn task_store_mongo_probe_contract_and_indexes() {
        let mongo_url = required("SKIFF_TASK_CONTROL_MONGO_URL");
        let database = std::env::var("SKIFF_TASK_CONTROL_MONGO_DB")
            .unwrap_or_else(|_| "skiff_task_control_probe".to_string());

        let store = MongoTaskStore::connect(&mongo_url, probe_options(&database))
            .await
            .expect("connect task store");
        store.ensure_indexes().await.expect("ensure indexes");

        let mut client_options = ClientOptions::parse(&mongo_url)
            .await
            .expect("parse probe client");
        client_options.app_name = Some("skiff-task-control-probe".to_string());
        let client = Client::with_options(client_options).expect("probe client");
        let mut indexes = client
            .database(&database)
            .collection::<mongodb::bson::Document>("tasks")
            .list_indexes()
            .await
            .expect("list indexes");
        let mut found = false;
        while let Some(index) = indexes.try_next().await.expect("index stream") {
            if index
                .options
                .as_ref()
                .and_then(|options| options.name.as_deref())
                == Some(TASK_STATE_DUE_AT_INDEX)
            {
                found = true;
                assert_eq!(
                    index.keys.get("state"),
                    Some(&mongodb::bson::Bson::Int32(1))
                );
                assert_eq!(
                    index.keys.get("dueAt"),
                    Some(&mongodb::bson::Bson::Int32(1))
                );
            }
        }
        assert!(found, "{TASK_STATE_DUE_AT_INDEX} index must exist");
        contract::run_contract(&store, &TestTime::WallClock).await;
        store.close().await.expect("close");
    }
}
