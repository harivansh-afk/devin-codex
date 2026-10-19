//! Private runtime configuration. Credentials never come from HTTP callers.
use rand::Rng;
use serde::Deserialize;
use std::{
    env, fs,
    path::{Path, PathBuf},
};
use thiserror::Error;

#[cfg(test)]
#[path = "../tests/unit/config.rs"]
mod tests;

#[derive(Error, Debug)]
#[error("{0}")]
pub struct ConfigError(pub &'static str);

// Deliberately not Debug: neither credentials nor the device ID belong in logs.
#[derive(Clone)]
pub struct Credentials {
    pub endpoint: String,
    pub api_key: String,
    pub device_id: String,
    pub client_version: String,
}

#[derive(Deserialize)]
struct StoredCredentials {
    windsurf_api_key: String,
    api_server_url: String,
}

fn home() -> Result<PathBuf, ConfigError> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or(ConfigError("HOME is not configured"))
}

pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex(&bytes)
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    bytes
        .iter()
        .flat_map(|b| {
            [
                DIGITS[(b >> 4) as usize] as char,
                DIGITS[(b & 15) as usize] as char,
            ]
        })
        .collect()
}

pub fn load_credentials(path: Option<&Path>) -> Result<Credentials, ConfigError> {
    let default;
    let path = match path {
        Some(path) => path,
        None => {
            let root = match env::var_os("XDG_DATA_HOME") {
                Some(value) => PathBuf::from(value),
                None => home()?.join(".local/share"),
            };
            default = root.join("devin/credentials.toml");
            &default
        }
    };
    let text = fs::read_to_string(path)
        .map_err(|_| ConfigError("Cannot read Devin credentials; sign in with Devin CLI first"))?;
    parse_credentials(&text)
}

fn parse_credentials(text: &str) -> Result<Credentials, ConfigError> {
    let stored: StoredCredentials =
        toml::from_str(text).map_err(|_| ConfigError("Invalid Devin credentials file"))?;
    if stored.windsurf_api_key.is_empty()
        || !stored
            .windsurf_api_key
            .bytes()
            .all(|b| (33..=126).contains(&b))
    {
        return Err(ConfigError("Invalid Devin API key"));
    }
    let url = reqwest::Url::parse(&stored.api_server_url)
        .map_err(|_| ConfigError("Invalid Devin deployment URL"))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || !matches!(url.port(), None | Some(443))
        || !matches!(
            url.host_str(),
            Some("server.enterprise.windsurf.com" | "server.codeium.com")
        )
    {
        return Err(ConfigError(
            "Unverified Devin deployment; expected a known HTTPS origin",
        ));
    }
    let mut device = [0u8; 366];
    rand::rng().fill_bytes(&mut device);
    Ok(Credentials {
        endpoint: format!(
            "{}exa.api_server_pb.ApiServerService/GetChatMessage",
            url.as_str()
        ),
        api_key: stored.windsurf_api_key,
        device_id: hex(&device),
        client_version: "3000.10.31".into(),
    })
}

pub fn load_local_token(path: Option<&Path>) -> Result<String, ConfigError> {
    let token = if let Some(path) = path {
        let file =
            fs::File::open(path).map_err(|_| ConfigError("Cannot read adapter token file"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = file
                .metadata()
                .map_err(|_| ConfigError("Cannot inspect adapter token file"))?;
            if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 {
                return Err(ConfigError("Adapter token file must be private (0600)"));
            }
        }
        use std::io::Read;
        let mut token = String::new();
        file.take(4097)
            .read_to_string(&mut token)
            .map_err(|_| ConfigError("Cannot read adapter token file"))?;
        token
    } else {
        env::var("DEVIN_CODEX_TOKEN")
            .map_err(|_| ConfigError("Set DEVIN_CODEX_TOKEN or --token-file"))?
    };
    let token = token.trim();
    if !(24..=4096).contains(&token.len()) || !token.bytes().all(|b| (33..=126).contains(&b)) {
        return Err(ConfigError(
            "Adapter token must contain 24..4096 printable ASCII characters",
        ));
    }
    Ok(token.into())
}

pub(crate) fn create_private_file(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

pub fn write_token(path: &Path) -> Result<(), ConfigError> {
    let mut file = create_private_file(path)
        .map_err(|_| ConfigError("Cannot create token file; existing files are never replaced"))?;
    use std::io::Write;
    writeln!(file, "{}", random_token()).map_err(|_| ConfigError("Cannot write token file"))
}
