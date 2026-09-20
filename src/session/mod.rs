//! Own the temporary listener, provider routing, and Codex process lifetime.
use crate::{
    Mode,
    config::{Credentials, random_token},
    gateway::GatewayClient,
    server,
};
use std::{ffi::OsString, path::Path};
use tokio::{net::TcpListener, process::Command, sync::oneshot};

mod catalog;
use catalog::Catalog;

#[cfg(test)]
#[path = "../../tests/unit/session.rs"]
mod tests;

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
        Self::start_with_mode(credentials, Mode::Strict).await
    }

    pub async fn start_with_mode(
        credentials: Credentials,
        mode: Mode,
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
        let router = server::router_with_mode(credentials, token.clone(), gateway, mode);
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
        let mut command = self.command(binary, args, Some(catalog.path()))?;
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
            status = child.wait() => {
                Ok(status.map_err(|_| "Cannot wait for Codex.")?.code().unwrap_or(1) as u8)
            }
            _ = shutdown() => {
                #[cfg(unix)]
                if let Some(group) = &group {
                    group.signal(nix::sys::signal::Signal::SIGTERM);
                } else {
                    let _ = nix::sys::signal::kill(
                        nix::unistd::Pid::from_raw(child.id().unwrap() as i32),
                        nix::sys::signal::Signal::SIGTERM,
                    );
                }
                #[cfg(not(unix))]
                let _ = child.start_kill();
                if tokio::time::timeout(std::time::Duration::from_secs(3), child.wait()).await.is_err() {
                    #[cfg(unix)]
                    if let Some(group) = &group {
                        group.signal(nix::sys::signal::Signal::SIGKILL);
                    }
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
        command.args(["--profile", "devin-codex"]);
        for setting in [
            "model_provider=\"devin\"".to_owned(),
            format!("model=\"{}\"", crate::compat::MODEL),
            format!(
                "model_providers.devin={{name=\"Devin\",base_url=\"{}\",wire_api=\"responses\",env_key=\"DEVIN_CODEX_SESSION_TOKEN\",requires_openai_auth=false,supports_websockets=false,request_max_retries=0,stream_max_retries=0}}",
                self.url
            ),
            "web_search=\"disabled\"".to_owned(),
            "service_tier=\"default\"".to_owned(),
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
