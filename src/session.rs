//! Temporary local server shared by live tests and Codex exec.
use crate::{
    config::{Credentials, random_token},
    gateway::GatewayClient,
    server,
};
use std::{
    ffi::OsString,
    io::Write,
    path::{Path, PathBuf},
};
use tokio::{net::TcpListener, process::Command, sync::oneshot};

#[cfg(unix)]
struct ProcessGroup(nix::unistd::Pid);

#[cfg(unix)]
impl ProcessGroup {
    fn signal(&self, signal: nix::sys::signal::Signal) {
        let _ = nix::sys::signal::killpg(self.0, signal);
    }
}

#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        self.signal(nix::sys::signal::Signal::SIGKILL);
    }
}

struct Catalog(PathBuf);

impl Catalog {
    async fn load(binary: &Path) -> Result<Self, &'static str> {
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            Command::new(binary)
                .args(["debug", "models", "--bundled"])
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| "Timed out reading Codex's bundled model catalog.")?
        .map_err(|_| "Cannot start Codex. Install it on PATH or use --codex.")?;
        let mut value: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(
            |_| "Cannot read Codex model catalog. Codex 0.155.1 or compatible is required.",
        )?;
        if !output.status.success()
            || !value["models"]
                .as_array()
                .is_some_and(|models| models.iter().any(|m| m["slug"] == "gpt-6-astra"))
        {
            return Err("Installed Codex does not provide the gpt-6-astra model catalog.");
        }
        value["models"]
            .as_array_mut()
            .unwrap()
            .retain(|m| m["slug"] == "gpt-6-astra");
        let bytes = serde_json::to_vec(&value).map_err(|_| "Cannot encode model catalog.")?;
        let path = std::env::temp_dir().join(format!("devin-codex-models-{}.json", random_token()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&path)
            .map_err(|_| "Cannot create temporary model catalog.")?;
        let catalog = Self(path);
        file.write_all(&bytes)
            .map_err(|_| "Cannot write temporary model catalog.")?;
        Ok(catalog)
    }
}

impl Drop for Catalog {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub struct Session {
    pub url: String,
    pub token: String,
    stop: Option<oneshot::Sender<()>>,
}

impl Session {
    pub async fn start(credentials: Credentials) -> Result<Self, &'static str> {
        Self::start_with_mode(credentials, false).await
    }

    pub async fn start_with_mode(
        credentials: Credentials,
        compatibility: bool,
    ) -> Result<Self, &'static str> {
        let gateway = GatewayClient::new(&credentials.endpoint, &credentials.api_key)
            .map_err(|_| "Cannot configure gateway transport.")?;
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| "Cannot bind loopback listener.")?;
        let url = format!(
            "http://{}/v1",
            listener
                .local_addr()
                .map_err(|_| "Cannot read listener address.")?
        );
        let token = random_token();
        let router = server::router_with_mode(credentials, token.clone(), gateway, compatibility);
        let (stop, stopped) = oneshot::channel();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await;
        });
        Ok(Self {
            url,
            token,
            stop: Some(stop),
        })
    }

    pub async fn run(&self, binary: &Path, args: &[OsString]) -> Result<u8, &'static str> {
        let catalog = Catalog::load(binary).await?;
        let mut command = self.command(binary, args, Some(&catalog.0))?;
        let detached = args.first().is_some_and(|a| a == "exec");
        #[cfg(unix)]
        if detached {
            use std::os::unix::process::CommandExt;
            command.as_std_mut().process_group(0);
        }
        let mut child = command
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| "Cannot start Codex. Install it on PATH or use --codex.")?;
        #[cfg(unix)]
        let group =
            detached.then(|| ProcessGroup(nix::unistd::Pid::from_raw(child.id().unwrap() as i32)));
        tokio::select! {
            status = child.wait() => Ok(status.map_err(|_| "Cannot wait for Codex.")?.code().unwrap_or(1) as u8),
            _ = shutdown() => {
                #[cfg(unix)]
                if let Some(group) = &group { group.signal(nix::sys::signal::Signal::SIGTERM); } else { let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(child.id().unwrap() as i32), nix::sys::signal::Signal::SIGTERM); }
                #[cfg(not(unix))]
                let _ = child.start_kill();
                if tokio::time::timeout(std::time::Duration::from_secs(3), child.wait()).await.is_err() {
                    #[cfg(unix)]
                    if let Some(group) = &group { group.signal(nix::sys::signal::Signal::SIGKILL); }
                    let _ = child.kill().await;
                }
                Ok(130)
            }
        }
    }

    fn command(
        &self,
        binary: &Path,
        args: &[OsString],
        catalog: Option<&Path>,
    ) -> Result<Command, &'static str> {
        // Keep routing under this launcher's control. No shell interprets arguments.
        for arg in args {
            let arg = arg.to_string_lossy();
            if [
                "-c",
                "--config",
                "-p",
                "--profile",
                "-m",
                "--model",
                "--oss",
                "--local-provider",
            ]
            .iter()
            .any(|flag| {
                arg == *flag
                    || arg.starts_with(&format!("{flag}="))
                    || (flag.len() == 2 && arg.starts_with(flag))
            }) {
                return Err(
                    "Provider, profile and model overrides are not supported by the launcher.",
                );
            }
        }
        let mut command = Command::new(binary);
        for setting in [
            "model_provider=\"devin\"".to_owned(),
            "model=\"gpt-6-astra\"".to_owned(),
            format!(
                "model_providers.devin={{name=\"Devin\",base_url=\"{}\",wire_api=\"responses\",env_key=\"DEVIN_CODEX_SESSION_TOKEN\",requires_openai_auth=false,supports_websockets=false,request_max_retries=0,stream_max_retries=0}}",
                self.url
            ),
            "web_search=\"disabled\"".to_owned(),
        ] {
            command.arg("-c").arg(setting);
        }
        if let Some(path) = catalog {
            command.arg("-c").arg(format!(
                "model_catalog_json={}",
                toml::Value::String(path.to_string_lossy().into_owned())
            ));
        }
        command
            .args(args)
            .env("DEVIN_CODEX_SESSION_TOKEN", &self.token);
        Ok(command)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

pub async fn shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
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
}
