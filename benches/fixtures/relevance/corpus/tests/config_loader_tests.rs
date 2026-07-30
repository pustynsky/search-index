#[test]
fn load_runtime_settings_reports_invalid_source() {
    let loader = ConfigLoader;
    let error = loader.load_runtime_settings("").unwrap_err();
    assert_eq!(error.message(), "config parse failed");
}

#[test]
fn config_loader_reads_max_retries() {
    let loader = ConfigLoader;
    let settings = loader.load_runtime_settings("MaxRetries=3").unwrap();
    assert_eq!(settings.max_retries, 3);
}

struct ConfigLoader;
