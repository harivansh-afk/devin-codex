//! Temporary local server shared by live tests and Codex exec.
use crate::{
    config::{Credentials, random_token},
    gateway::GatewayClient,
    server,
};
use std::{ffi::OsString, path::Path};
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

pub struct Session {
    pub url: String,
    pub token: String,
    stop: Option<oneshot::Sender<()>>,
}

impl Session {
    pub async fn start(credentials: Credentials) -> Result<Self, &'static str> {
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
        let router = server::router(credentials, token.clone(), gateway);
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
        let mut command = self.command(binary, args)?;
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.as_std_mut().process_group(0);
        }
        let mut child = command
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| "Cannot start Codex. Install it on PATH or use --codex.")?;
        #[cfg(unix)]
        let group = ProcessGroup(nix::unistd::Pid::from_raw(child.id().unwrap() as i32));
        tokio::select! {
            status = child.wait() => Ok(status.map_err(|_| "Cannot wait for Codex.")?.code().unwrap_or(1) as u8),
            _ = shutdown() => {
                #[cfg(unix)]
                group.signal(nix::sys::signal::Signal::SIGTERM);
                #[cfg(not(unix))]
                let _ = child.start_kill();
                if tokio::time::timeout(std::time::Duration::from_secs(3), child.wait()).await.is_err() {
                    #[cfg(unix)]
                    group.signal(nix::sys::signal::Signal::SIGKILL);
                    let _ = child.kill().await;
                }
                Ok(130)
            }
        }
    }

    fn command(&self, binary: &Path, args: &[OsString]) -> Result<Command, &'static str> {
        if args.first().and_then(|arg| arg.to_str()) != Some("exec") {
            return Err(
                "Only Codex exec is supported. Use: devin-codex run -- exec 'task'. Interactive auth is not verified.",
            );
        }
        // Keep routing under this launcher's control. No shell interprets arguments.
        for arg in &args[1..] {
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
        command
            .arg("exec")
            .args(["-m", "gpt-6-astra", "-c", "model_provider=\"openai\"", "-c"])
            .arg(format!("openai_base_url=\"{}\"", self.url))
            .args(&args[1..])
            .env("CODEX_API_KEY", &self.token);
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
            )
            .unwrap();
        let command = command.as_std();
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(
            *args.last().unwrap(),
            std::ffi::OsStr::new("a task with spaces")
        );
        assert!(command.get_envs().any(|(key, value)| key == "CODEX_API_KEY"
            && value == Some(std::ffi::OsStr::new(&session.token))));
        for args in [
            vec![],
            vec!["resume".into()],
            vec!["exec".into(), "-cmodel_provider=other".into()],
        ] {
            assert!(session.command(Path::new("codex"), &args).is_err());
        }
        drop(session);
        tokio::task::yield_now().await;
        assert!(client.get(url).send().await.is_err());
    }
}
