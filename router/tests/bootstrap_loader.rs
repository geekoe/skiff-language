//! W-bootstrap blocking loader tests: bounded concurrency, fail-closed
//! saturation, per-operation deadline, shutdown drain/refusal and health
//! counters (C-bootstrap §2.3/§3.8).

use std::sync::{Arc, Barrier};
use std::time::Duration;

use skiff_router::bootstrap::{BlockingLoader, BlockingLoaderError, BlockingLoaderOptions};

fn loader_with(concurrency: usize, read_deadline: Duration) -> Arc<BlockingLoader> {
    Arc::new(BlockingLoader::new(BlockingLoaderOptions {
        concurrency,
        read_deadline,
        drain_deadline: Duration::from_secs(2),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn saturation_fails_closed_without_queueing() {
        let loader = loader_with(2, Duration::from_secs(5));
        let barrier = Arc::new(Barrier::new(3));
        let first_barrier = Arc::clone(&barrier);
        let first = tokio::spawn({
            let loader = Arc::clone(&loader);
            async move {
                loader
                    .run(move || {
                        first_barrier.wait();
                        Ok::<(), String>(())
                    })
                    .await
            }
        });
        let second_barrier = Arc::clone(&barrier);
        let second = tokio::spawn({
            let loader = Arc::clone(&loader);
            async move {
                loader
                    .run(move || {
                        second_barrier.wait();
                        Ok::<(), String>(())
                    })
                    .await
            }
        });
        while loader.health().occupancy < 2 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        let saturated = loader
            .run(|| Ok::<(), String>(()))
            .await
            .expect_err("third op must be saturated");
        assert_eq!(saturated, BlockingLoaderError::Saturated);
        let saturated = loader
            .run(|| Ok::<(), String>(()))
            .await
            .expect_err("fourth op must be saturated");
        assert_eq!(saturated, BlockingLoaderError::Saturated);
        assert_eq!(loader.health().saturated, 2);
        assert_eq!(loader.health().queued, 0);
        assert_eq!(loader.health().occupancy, 2);

        barrier.wait();
        let first_result = tokio::time::timeout(Duration::from_secs(5), first)
            .await
            .expect("first holder must finish after barrier release");
        assert!(first_result.expect("first holder join").is_ok());
        let second_result = tokio::time::timeout(Duration::from_secs(5), second)
            .await
            .expect("second holder must finish after barrier release");
        assert!(second_result.expect("second holder join").is_ok());
        assert_eq!(loader.health().occupancy, 0);
    }

    #[tokio::test]
    async fn deadline_aborts_fail_closed() {
        let loader = loader_with(1, Duration::from_millis(10));
        let error = loader
            .run(move || {
                std::thread::sleep(Duration::from_millis(200));
                Ok::<(), String>(())
            })
            .await
            .expect_err("sleep must exceed the deadline");
        assert_eq!(error, BlockingLoaderError::Deadline);
        assert_eq!(loader.health().deadline_aborts, 1);
    }

    #[tokio::test]
    async fn shutdown_refuses_new_loads_and_drains_occupancy() {
        let loader = loader_with(1, Duration::from_secs(5));
        let op = tokio::spawn({
            let loader = Arc::clone(&loader);
            async move {
                loader
                    .run(move || {
                        std::thread::sleep(Duration::from_millis(80));
                        Ok::<u64, String>(7)
                    })
                    .await
            }
        });
        // Give the in-flight op a chance to start before draining.
        tokio::time::sleep(Duration::from_millis(20)).await;
        loader.shutdown().await;
        let refused = loader
            .run(|| Ok::<(), String>(()))
            .await
            .expect_err("new load after shutdown must be refused");
        assert_eq!(refused, BlockingLoaderError::Shutdown);
        assert!(loader.health().shutdown);
        assert_eq!(loader.health().shutdown_refusals, 1);
        assert_eq!(loader.health().occupancy, 0);
        assert_eq!(op.await.expect("op join").expect("op result"), 7);
    }

    #[tokio::test]
    async fn operation_errors_are_typed_and_health_tracks_them() {
        let loader = loader_with(2, Duration::from_secs(5));
        let error = loader
            .run(|| Err::<(), _>("boom".to_string()))
            .await
            .expect_err("operation must fail");
        assert!(matches!(
            error,
            BlockingLoaderError::Operation(message) if message == "boom"
        ));
        assert_eq!(loader.health().saturated, 0);
        assert_eq!(loader.health().deadline_aborts, 0);
    }
}
