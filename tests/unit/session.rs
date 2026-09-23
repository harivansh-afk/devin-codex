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
async fn launch_defaults_to_an_available_model_without_astra() {
    let models = Models::parse(&serde_json::json!({"families":[{"variants":[
        {"model_uid":"claude-opus-5-5-high","label":"Claude Opus High"}
    ]}]}))
    .unwrap();
    let session = Session::start_with_models(credentials(), Mode::Codex, models)
        .await
        .unwrap();
    let command = session
        .command(Path::new("codex"), &[], None, &RunOptions::default())
        .unwrap();
    let args: Vec<_> = command.as_std().get_args().collect();
    assert!(args.contains(&std::ffi::OsStr::new(
        "model=\"devin-claude-opus-5-5-high\""
    )));
    assert_eq!(RunOptions::default().devin, Path::new("devin"));
    for model in ["claude-opus-5-5-high", "devin-claude-opus-5-5-high"] {
        let options = RunOptions {
            model: Some(model.into()),
            ..Default::default()
        };
        assert!(
            session
                .command(Path::new("codex"), &[], None, &options)
                .unwrap()
                .as_std()
                .get_args()
                .any(|arg| arg == "model=\"devin-claude-opus-5-5-high\"")
        );
    }
    let options = RunOptions {
        model: Some("unknown".into()),
        ..Default::default()
    };
    assert!(
        session
            .command(Path::new("codex"), &[], None, &options)
            .is_err()
    );
    let options = RunOptions {
        model: Some("claude-opus-5-5-high".into()),
        reasoning_effort: Some("low".into()),
        ..Default::default()
    };
    assert!(
        session
            .command(Path::new("codex"), &[], None, &options)
            .is_err()
    );
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
            &RunOptions::default(),
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
        assert!(
            session
                .command(Path::new("codex"), &args, None, &RunOptions::default())
                .is_err()
        );
    }
    assert!(
        session
            .command(Path::new("codex"), &[], None, &RunOptions::default())
            .is_ok()
    );
    assert!(
        session
            .command(
                Path::new("codex"),
                &["resume".into()],
                None,
                &RunOptions::default()
            )
            .is_ok()
    );
    drop(session);
    tokio::task::yield_now().await;
    assert!(client.get(url).send().await.is_err());
}

#[tokio::test]
async fn reasoning_presets_preserve_routing_and_reject_unknown_efforts() {
    let session = Session::start(credentials()).await.unwrap();
    for effort in crate::compat::EFFORTS {
        let options = RunOptions {
            reasoning_effort: Some((*effort).into()),
            ..Default::default()
        };
        let command = session
            .command(Path::new("codex"), &[], None, &options)
            .unwrap();
        let args: Vec<_> = command.as_std().get_args().collect();
        assert!(args.contains(&std::ffi::OsStr::new(&format!(
            "model_reasoning_effort=\"{effort}\""
        ))));
        assert!(args.contains(&std::ffi::OsStr::new("model_provider=\"devin\"")));
        assert!(
            session
                .command(
                    Path::new("codex"),
                    &["-c".into(), "model_provider=other".into()],
                    None,
                    &options
                )
                .is_err()
        );
    }
    let options = RunOptions {
        reasoning_effort: Some("invalid".into()),
        ..Default::default()
    };
    assert!(
        session
            .command(Path::new("codex"), &[], None, &options)
            .is_err()
    );
}

#[tokio::test]
async fn trusted_project_paths_are_resolved_and_toml_escaped() {
    let session = Session::start(credentials()).await.unwrap();
    let path = std::env::temp_dir().join(format!("devin-project-\"{}", random_token()));
    std::fs::create_dir(&path).unwrap();
    let options = RunOptions {
        trust_project: Some(path.clone()),
        ..Default::default()
    };
    let command = session
        .command(Path::new("codex"), &[], None, &options)
        .unwrap();
    let setting = command
        .as_std()
        .get_args()
        .find(|arg| arg.to_string_lossy().starts_with("projects="))
        .unwrap();
    let parsed: toml::Value = toml::from_str(&setting.to_string_lossy()).unwrap();
    assert_eq!(
        parsed["projects"][path.canonicalize().unwrap().to_str().unwrap()]["trust_level"].as_str(),
        Some("trusted")
    );
    assert_eq!(parsed["projects"].as_table().unwrap().len(), 1);
    std::fs::remove_dir(&path).unwrap();
    assert!(
        session
            .command(Path::new("codex"), &[], None, &options)
            .is_err()
    );
    let options = RunOptions {
        trust_project: Some(std::env::current_exe().unwrap()),
        ..Default::default()
    };
    assert!(
        session
            .command(Path::new("codex"), &[], None, &options)
            .is_err()
    );
}
