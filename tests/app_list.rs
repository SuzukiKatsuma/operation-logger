#[cfg(target_os = "windows")]
#[test]
fn list_running_applications_does_not_fail() {
    let result = operation_logger::list_running_applications();
    assert!(
        result.is_ok(),
        "list_running_applications failed: {:?}",
        result.err()
    );

    let windows = result.unwrap();

    for w in windows {
        assert!(!w.title.trim().is_empty());
    }
}
