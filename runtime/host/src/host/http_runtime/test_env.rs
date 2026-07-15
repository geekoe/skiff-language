use std::{ffi::OsString, future::Future, sync::OnceLock};

use tokio::sync::Mutex;

static HTTP_EGRESS_TEST_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

struct HttpEgressTestEnvGuard {
    previous: Vec<(&'static str, Option<OsString>)>,
}

impl HttpEgressTestEnvGuard {
    fn apply(overrides: impl IntoIterator<Item = (&'static str, Option<OsString>)>) -> Self {
        let mut guard = Self {
            previous: Vec::new(),
        };

        for (name, value) in overrides {
            guard.previous.push((name, std::env::var_os(name)));
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }

        guard
    }
}

impl Drop for HttpEgressTestEnvGuard {
    fn drop(&mut self) {
        while let Some((name, value)) = self.previous.pop() {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}

pub(super) async fn with_http_egress_env_overrides_for_test<R>(
    overrides: impl IntoIterator<Item = (&'static str, Option<OsString>)>,
    f: impl Future<Output = R>,
) -> R {
    let lock = HTTP_EGRESS_TEST_ENV_LOCK.get_or_init(|| Mutex::new(()));
    let _lock_guard = lock.lock().await;
    // Declared after the lock guard so environment restoration always runs before unlock.
    let _env_guard = HttpEgressTestEnvGuard::apply(overrides);

    f.await
}
