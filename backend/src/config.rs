use std::{env, net::SocketAddr, path::PathBuf, time::Duration};

/// Runtime configuration, read from environment variables.
/// Everything that changes at runtime (printers, keys, accounts) lives in the
/// data store instead, so the container can be configured from the UI.
#[derive(Debug, Clone)]
pub struct Config {
    /// Address to listen on. `BIND`, default `0.0.0.0:8080`.
    pub bind: SocketAddr,
    /// Persistent data (state file, downloads, sliced files). `DATA_DIR`, default `./data`.
    pub data_dir: PathBuf,
    /// Built frontend. `STATIC_DIR`, default `./web`.
    pub static_dir: PathBuf,
    /// Optional sub-path when served behind a reverse proxy, e.g. `/airslice`.
    /// `BASE_PATH`, default empty (served at `/`).
    pub base_path: String,
    /// Optional fixed admin password. When unset, the first visitor sets one in the UI.
    pub admin_password: Option<String>,
    /// OrcaSlicer executable (AppRun of an extracted AppImage). `ORCA_SLICER_PATH`.
    pub orca_path: PathBuf,
    /// OrcaSlicer system profiles (`resources/profiles`). `ORCA_PROFILES_DIR`.
    pub orca_profiles_dir: PathBuf,
    /// Force the `Secure` flag on the session cookie. `COOKIE_SECURE=true|false`.
    /// When unset it is decided per request from `X-Forwarded-Proto`.
    pub cookie_secure: Option<bool>,
    /// Max time a single slice may take. `SLICE_TIMEOUT_SECS`, default 900.
    pub slice_timeout: Duration,
    /// Keep at most this many jobs in history. `MAX_JOBS`, default 50.
    pub max_jobs: usize,
    /// Home Assistant add-on: requests from this address that carry
    /// `X-Ingress-Path` are already authenticated by Home Assistant.
    /// `INGRESS_TRUSTED_PEER`, e.g. `172.30.32.2`. Unset outside HA.
    pub ingress_peer: Option<std::net::IpAddr>,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let bind = env::var("BIND").unwrap_or_else(|_| "0.0.0.0:8080".into()).parse()?;
        let data_dir = PathBuf::from(env::var("DATA_DIR").unwrap_or_else(|_| "./data".into()));
        let static_dir = PathBuf::from(env::var("STATIC_DIR").unwrap_or_else(|_| "./web".into()));
        let base_path = normalize_base_path(&env::var("BASE_PATH").unwrap_or_default());
        let admin_password = env::var("ADMIN_PASSWORD").ok().filter(|s| !s.is_empty());
        let orca_path = PathBuf::from(
            env::var("ORCA_SLICER_PATH").unwrap_or_else(|_| "/opt/orcaslicer/AppRun".into()),
        );
        let orca_profiles_dir = env::var("ORCA_PROFILES_DIR").map(PathBuf::from).unwrap_or_else(|_| {
            orca_path
                .parent()
                .map(|p| p.join("resources").join("profiles"))
                .unwrap_or_else(|| PathBuf::from("/opt/orcaslicer/resources/profiles"))
        });
        let cookie_secure = env::var("COOKIE_SECURE").ok().and_then(|v| match v.to_lowercase().as_str() {
            "1" | "true" | "yes" => Some(true),
            "0" | "false" | "no" => Some(false),
            _ => None,
        });
        let slice_timeout = Duration::from_secs(
            env::var("SLICE_TIMEOUT_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(900),
        );
        let max_jobs = env::var("MAX_JOBS").ok().and_then(|v| v.parse().ok()).unwrap_or(50);
        let ingress_peer = match env::var("INGRESS_TRUSTED_PEER") {
            Ok(v) if !v.trim().is_empty() => Some(v.trim().parse().map_err(|_| anyhow::anyhow!("INGRESS_TRUSTED_PEER must be an IP address"))?),
            _ => None,
        };
        Ok(Self {
            bind,
            data_dir,
            static_dir,
            base_path,
            admin_password,
            orca_path,
            orca_profiles_dir,
            cookie_secure,
            slice_timeout,
            max_jobs,
            ingress_peer,
        })
    }

    pub fn jobs_dir(&self) -> PathBuf {
        self.data_dir.join("jobs")
    }
}

/// `"airslice/"` -> `"/airslice"`, `"/"` -> `""`.
pub fn normalize_base_path(raw: &str) -> String {
    let trimmed = raw.trim().trim_matches('/');
    if trimmed.is_empty() { String::new() } else { format!("/{trimmed}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_path_normalization() {
        assert_eq!(normalize_base_path(""), "");
        assert_eq!(normalize_base_path("/"), "");
        assert_eq!(normalize_base_path("airslice"), "/airslice");
        assert_eq!(normalize_base_path("/airslice/"), "/airslice");
        assert_eq!(normalize_base_path(" /a/b/ "), "/a/b");
    }
}
