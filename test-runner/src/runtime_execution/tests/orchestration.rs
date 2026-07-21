use std::cell::Cell;

use super::*;

#[test]
fn business_success_is_sent_exactly_once() {
    let calls = Cell::new(0);
    let result = execute_business_request_once(|| {
        calls.set(calls.get() + 1);
        Ok(http::HttpResponse {
            status: 200,
            body: "ok".to_string(),
        })
    });

    assert_eq!(result, (true, None));
    assert_eq!(calls.get(), 1);
}

#[test]
fn business_503_is_sent_exactly_once() {
    let calls = Cell::new(0);
    let result = execute_business_request_once(|| {
        calls.set(calls.get() + 1);
        Ok(http::HttpResponse {
            status: 503,
            body: "runtime unavailable".to_string(),
        })
    });

    assert!(!result.0);
    assert!(result.1.unwrap().contains("HTTP 503"));
    assert_eq!(calls.get(), 1);
}

#[test]
fn business_timeout_and_transport_errors_are_each_sent_exactly_once() {
    for kind in [
        std::io::ErrorKind::TimedOut,
        std::io::ErrorKind::ConnectionReset,
    ] {
        let calls = Cell::new(0);
        let result = execute_business_request_once(|| {
            calls.set(calls.get() + 1);
            Err(CanonicalFixtureError::Io {
                path: "http://127.0.0.1/test".to_string(),
                source: std::io::Error::new(kind, "scripted transport failure"),
            })
        });

        assert!(!result.0);
        assert_eq!(calls.get(), 1);
    }
}
