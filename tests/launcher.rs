#![cfg(unix)]
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use std::{fs, os::unix::fs::PermissionsExt, process::Stdio, time::Duration};
use tokio::process::Command;

#[tokio::test]
async fn forwards_exit_status_and_stops_child_group_on_signal() {
    let dir = std::env::temp_dir().join(format!("devin-codex-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&dir).unwrap();
    let credentials = dir.join("credentials.toml");
    fs::write(&credentials, "windsurf_api_key='synthetic-test-key'\napi_server_url='https://server.enterprise.windsurf.com'\n").unwrap();
    let shell = Command::new("sh")
        .args(["-c", "command -v sh"])
        .output()
        .await
        .unwrap();
    let shell = String::from_utf8(shell.stdout).unwrap();
    let script = dir.join("codex");
    let prefix = format!(
        "#!{}\nif [ \"$1\" = models ]; then echo '{{\"families\":[{{\"variants\":[{{\"model_uid\":\"claude-opus-5-5-high\",\"label\":\"Claude Opus High\"}}]}}]}}'; exit 0; fi\nif [ \"$1\" = debug ]; then echo '{{\"models\":[{{\"slug\":\"gpt-6-astra\",\"supported_reasoning_levels\":[{{\"effort\":\"medium\"}}]}}]}}'; exit 0; fi\n[ \"$1\" = --profile ] && [ \"$2\" = devin-codex ] || exit 9\n",
        shell.trim()
    );
    fs::write(&script, format!("{prefix}exit 7\n")).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_devin-codex"));
    command
        .args([
            "--credentials",
            credentials.to_str().unwrap(),
            "run",
            "--devin",
            script.to_str().unwrap(),
            "--codex",
            script.to_str().unwrap(),
            "--",
            "exec",
            "task",
        ])
        .env("DEVIN_CODEX_TEST_DIR", &dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    assert_eq!(command.status().await.unwrap().code(), Some(7));

    fs::write(&script, format!("{prefix}trap 'touch \"$DEVIN_CODEX_TEST_DIR/stopped\"; exit 0' TERM\nsleep 60 &\necho $$ > \"$DEVIN_CODEX_TEST_DIR/ready\"\nwait\n")).unwrap();
    let mut child = command.spawn().unwrap();
    let ready = dir.join("ready");
    tokio::time::timeout(Duration::from_secs(5), async {
        while !ready.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let group = Pid::from_raw(fs::read_to_string(ready).unwrap().trim().parse().unwrap());
    kill(Pid::from_raw(child.id().unwrap() as i32), Signal::SIGTERM).unwrap();
    let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status.code(), Some(130));
    assert!(
        dir.join("stopped").exists(),
        "Codex must receive a graceful signal"
    );
    // The leader must be reaped; the wrapper owns and terminates its group.
    assert!(kill(group, None).is_err());
    fs::remove_dir_all(dir).unwrap();
}
