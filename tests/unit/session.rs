use super::*;

fn credentials() -> Credentials {
    Credentials {
        endpoint: "http://127.0.0.1:1/rpc".into(),
        api_key: "test-key".into(),
        device_id: String::new(),
        client_version: String::new(),
    }
}

#[tokio::test]
async fn private_listener_and_command_lifecycle() {
    let session = Session::start(credentials()).await.unwrap();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap();
    let url = session.url.replace("/v1", "/healthz");
    assert_eq!(client.get(&url).send().await.unwrap().status(), 401);
    assert_eq!(
        client
            .get(&url)
            .bearer_auth(&session.token)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let command = session
        .command(
            Path::new("codex"),
            &["exec".into(), "a task with spaces".into()],
            None,
        )
        .unwrap();
    let command = command.as_std();
    let args: Vec<_> = command.get_args().collect();
    assert_eq!(
        *args.last().unwrap(),
        std::ffi::OsStr::new("a task with spaces")
    );
    assert!(
        command
            .get_envs()
            .any(|(key, value)| key == "DEVIN_CODEX_SESSION_TOKEN"
                && value == Some(std::ffi::OsStr::new(&session.token)))
    );
    for args in [
        vec!["exec".into(), "-cmodel_provider=other".into()],
        vec!["--profile=other".into()],
    ] {
        assert!(session.command(Path::new("codex"), &args, None).is_err());
    }
    assert!(session.command(Path::new("codex"), &[], None).is_ok());
    assert!(
        session
            .command(Path::new("codex"), &["resume".into()], None)
            .is_ok()
    );
    drop(session);
    tokio::task::yield_now().await;
    assert!(client.get(url).send().await.is_err());
}
