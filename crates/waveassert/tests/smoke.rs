//! Smoke test for the library crate: it builds and its tests run.

#[test]
fn crate_reports_its_unpublished_version() {
    assert_eq!(env!("CARGO_PKG_VERSION"), "0.0.0");
}
