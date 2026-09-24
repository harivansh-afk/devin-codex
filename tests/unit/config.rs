use super::*;

#[test]
fn validates_deployment_and_never_echoes_secrets() {
    for origin in [
        "http://server.codeium.com",
        "https://example.org",
        "https://key@server.codeium.com",
        "https://server.codeium.com/?key=secret",
        "https://server.codeium.com:444",
    ] {
        let input = format!("windsurf_api_key='private-test-key'\napi_server_url='{origin}'");
        let error = parse_credentials(&input).err().expect("reject origin");
        assert!(!error.to_string().contains("private-test-key"));
    }
    let credentials = parse_credentials("windsurf_api_key='private-test-key'\napi_server_url='https://server.enterprise.windsurf.com'").unwrap();
    assert!(
        credentials
            .endpoint
            .starts_with("https://server.enterprise.windsurf.com/")
    );
    assert_eq!(credentials.device_id.len(), 732);
    assert_eq!(random_token().len(), 64);
}

#[test]
fn private_files_never_overwrite_existing_state() {
    let path = std::env::temp_dir().join(format!("devin-private-test-{}", random_token()));
    write_token(&path).unwrap();
    let original = std::fs::read(&path).unwrap();
    assert!(write_token(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    std::fs::remove_file(path).unwrap();
}
