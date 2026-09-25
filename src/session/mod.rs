//! Own the temporary listener, provider routing, and Codex process lifetime.
use crate::{
    Mode,
    config::{Credentials, random_token},
    gateway::GatewayClient,
    models::Models,
    server,
};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};
use tokio::{net::TcpListener, process::Command, sync::oneshot};

mod catalog;
use catalog::Catalog;

#[cfg(test)]
#[path = "../../tests/unit/session.rs"]
mod tests;

#[derive(clap::Args)]
pub struct RunOptions {
    /// Devin CLI used to list models for this account.
    #[arg(long, default_value = "devin")]
    pub devin: PathBuf,
    /// Select one of the curated Devin model families (or its catalog alias).
    #[arg(long)]
    pub model: Option<String>,
    /// Override reasoning effort for this session without changing the saved profile.
    #[arg(long, value_parser = clap::builder::PossibleValuesParser::new(crate::compat::EFFORTS.iter().copied()))]
    pub reasoning_effort: Option<String>,
    /// Trust this project directory for this session.
    #[arg(long)]
    pub trust_project: Option<PathBuf>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            devin: PathBuf::from("devin"),
            model: None,
            reasoning_effort: None,
            trust_project: None,
        }
    }
}

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
    models: Models,
}

impl Session {
    pub async fn start(
        credentials: Credentials,
        mode: Mode,
        models: Models,
    ) -> Result<Self, &'static str> {
        let gateway = GatewayClient::new(&credentials.endpoint, &credentials.api_key)
            .map_err(|_| "Cannot configure gateway transport.")?
            .with_diagnostics();
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
        let router = server::router(credentials, token.clone(), gateway, mode, models.clone());
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
            models,
        })
    }

    pub async fn run(
        &self,
        binary: &Path,
        args: &[OsString],
        options: &RunOptions,
    ) -> Result<u8, &'static str> {
        let catalog = Catalog::load(binary, &self.models).await?;
        let mut command = self.command(binary, args, Some(catalog.path()), options)?;
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
        options: &RunOptions,
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
        let default_model = if self.models.entries().is_empty()
            || self.models.model(crate::compat::MODEL).is_some()
        {
            crate::compat::MODEL
        } else {
            &self.models.entries()[0].alias
        };
        let selected_model = match options.model.as_deref() {
            None => default_model,
            Some(model) if model == crate::compat::MODEL && self.models.entries().is_empty() => {
                model
            }
            Some(model) => self
                .models
                .model(model)
                .map(|entry| entry.alias.as_str())
                .ok_or("Model is not in the selected Devin catalog.")?,
        };
        let selected = self.models.model(selected_model);
        let effort = options.reasoning_effort.as_deref().or_else(|| {
            selected
                .filter(|model| model.variants.len() == 1)
                .map(|model| model.default_effort())
        });
        if let Some(effort) = effort {
            if !crate::compat::EFFORTS.contains(&effort) {
                return Err("Unsupported reasoning effort.");
            }
            if selected.is_some_and(|model| !model.variants.contains_key(effort)) {
                return Err("Reasoning effort is unavailable for the selected Devin model.");
            }
        }
        // The adapter retries whole buffered generations; disable nested Codex retries.
        for setting in [
            "model_provider=\"devin\"".to_owned(),
            format!("model={}", toml::Value::String(selected_model.into())),
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
        if let Some(effort) = effort {
            command.arg("-c").arg(format!(
                "model_reasoning_effort={}",
                toml::Value::String(effort.into())
            ));
        }
        if let Some(path) = &options.trust_project {
            let path = path
                .canonicalize()
                .map_err(|_| "Cannot resolve trusted project directory.")?;
            if !path.is_dir() {
                return Err("Trusted project must be a directory.");
            }
            command.arg("-c").arg(format!(
                "projects={{{}={{trust_level=\"trusted\"}}}}",
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
