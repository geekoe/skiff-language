use std::{ffi::OsString, time::Duration};

use crate::{
    capability_context::HTTP_REQUEST_ADMIN_OVERRIDE_ENV,
    host::http_runtime::{
        egress::with_http_admin_unsafe_override_for_test,
        test_env::with_http_egress_env_overrides_for_test,
    },
};

use super::helpers::{with_http_proxy_env_for_test, HTTP_PROXY_ENV_NAMES};

const TEST_ENV_LOCK_TIMEOUT: Duration = Duration::from_secs(5);

async fn snapshot_http_egress_env(names: &[&'static str]) -> Vec<(&'static str, Option<OsString>)> {
    tokio::time::timeout(
        TEST_ENV_LOCK_TIMEOUT,
        with_http_egress_env_overrides_for_test(std::iter::empty(), async {
            names
                .iter()
                .map(|name| (*name, std::env::var_os(name)))
                .collect()
        }),
    )
    .await
    .expect("HTTP egress test environment lock should remain available")
}

fn proxy_url_distinct_from(snapshot: &[(&'static str, Option<OsString>)]) -> String {
    for suffix in 0..=HTTP_PROXY_ENV_NAMES.len() {
        let candidate = format!("http://panic-proxy-{suffix}.invalid:8080");
        let candidate_os = OsString::from(&candidate);
        if snapshot
            .iter()
            .filter(|(name, _)| !matches!(*name, "NO_PROXY" | "no_proxy"))
            .all(|(_, value)| value.as_ref() != Some(&candidate_os))
        {
            return candidate;
        }
    }

    unreachable!("more proxy URL candidates than ambient proxy values")
}

#[tokio::test]
async fn admin_unsafe_override_restores_environment_after_panic() {
    let before = snapshot_http_egress_env(&[HTTP_REQUEST_ADMIN_OVERRIDE_ENV]).await;
    let allow_unsafe_targets = before[0].1.is_none();
    let expected_override = allow_unsafe_targets.then(|| OsString::from("true"));
    let panic_expected_override = expected_override.clone();

    let panic = tokio::spawn(async move {
        with_http_admin_unsafe_override_for_test(allow_unsafe_targets, async move {
            assert_eq!(
                std::env::var_os(HTTP_REQUEST_ADMIN_OVERRIDE_ENV),
                panic_expected_override
            );
            panic!("panic inside admin unsafe override helper");
        })
        .await;
    })
    .await
    .expect_err("admin unsafe override helper future should panic");
    assert!(panic.is_panic(), "spawned helper task should report panic");

    assert_eq!(
        snapshot_http_egress_env(&[HTTP_REQUEST_ADMIN_OVERRIDE_ENV]).await,
        before
    );

    tokio::time::timeout(
        TEST_ENV_LOCK_TIMEOUT,
        with_http_admin_unsafe_override_for_test(allow_unsafe_targets, async move {
            assert_eq!(
                std::env::var_os(HTTP_REQUEST_ADMIN_OVERRIDE_ENV),
                expected_override
            );
        }),
    )
    .await
    .expect("admin unsafe override helper should remain usable after panic");
    assert_eq!(
        snapshot_http_egress_env(&[HTTP_REQUEST_ADMIN_OVERRIDE_ENV]).await,
        before
    );
}

#[tokio::test]
async fn proxy_environment_restores_all_variables_after_panic() {
    let before = snapshot_http_egress_env(&HTTP_PROXY_ENV_NAMES).await;
    let proxy_url = proxy_url_distinct_from(&before);
    let panic_proxy_url = proxy_url.clone();

    let panic = tokio::spawn(async move {
        let expected_proxy = OsString::from(&panic_proxy_url);
        with_http_proxy_env_for_test(&panic_proxy_url, async move {
            for name in HTTP_PROXY_ENV_NAMES {
                let expected =
                    (!matches!(name, "NO_PROXY" | "no_proxy")).then(|| expected_proxy.clone());
                assert_eq!(std::env::var_os(name), expected, "override for {name}");
            }
            panic!("panic inside proxy environment helper");
        })
        .await;
    })
    .await
    .expect_err("proxy environment helper future should panic");
    assert!(panic.is_panic(), "spawned helper task should report panic");

    assert_eq!(
        snapshot_http_egress_env(&HTTP_PROXY_ENV_NAMES).await,
        before
    );

    let expected_proxy = OsString::from(&proxy_url);
    tokio::time::timeout(
        TEST_ENV_LOCK_TIMEOUT,
        with_http_proxy_env_for_test(&proxy_url, async move {
            for name in HTTP_PROXY_ENV_NAMES {
                let expected =
                    (!matches!(name, "NO_PROXY" | "no_proxy")).then(|| expected_proxy.clone());
                assert_eq!(std::env::var_os(name), expected, "override for {name}");
            }
        }),
    )
    .await
    .expect("proxy environment helper should remain usable after panic");
    assert_eq!(
        snapshot_http_egress_env(&HTTP_PROXY_ENV_NAMES).await,
        before
    );
}
