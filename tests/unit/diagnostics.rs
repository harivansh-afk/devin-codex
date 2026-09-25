use super::*;

#[test]
fn log_is_private_append_only_and_bounded() {
    let dir = std::env::temp_dir().join(format!("devin-diagnostics-{}", uuid::Uuid::new_v4()));
    let path = dir.join("gateway.jsonl");
    let value = json!({"code":"unknown", "attempt":1});
    append(&path, &value).unwrap();
    append(&path, &value).unwrap();
    let lines = fs::read_to_string(&path).unwrap();
    assert_eq!(lines.lines().count(), 2);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(MAX_LOG_BYTES)
        .unwrap();
    append(&path, &value).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().len(), MAX_LOG_BYTES);
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn log_does_not_follow_symlinks() {
    let dir = std::env::temp_dir().join(format!("devin-diagnostics-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&dir).unwrap();
    let target = dir.join("target");
    fs::write(&target, "untouched").unwrap();
    let path = dir.join("gateway.jsonl");
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert!(append(&path, &json!({"code":"unknown"})).is_err());
    assert_eq!(fs::read_to_string(target).unwrap(), "untouched");
    fs::remove_dir_all(dir).unwrap();
}
