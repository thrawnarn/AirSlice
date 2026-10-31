//! Printer camera pictures: straight from the printer (P1/A1, TCP 6000) or
//! through a Home Assistant camera entity, which leaves the printer's single
//! camera connection to the Home Assistant integration.

use crate::{
    models::{Printer, now},
    state::Shared,
    tls::printer_client_config,
};
use anyhow::{Context, anyhow, bail};
use std::{collections::HashMap, sync::Arc, sync::Mutex, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};
use tokio_rustls::TlsConnector;

/// One picture per printer per two seconds, however many pages look.
/// Keyed by printer and source, so changing the source never shows the old one.
type Pictures = HashMap<String, (i64, Arc<Vec<u8>>)>;
static CACHE: Mutex<Option<Pictures>> = Mutex::new(None);
const FRESH_SECS: i64 = 2;

/// The latest picture from the printer's camera (JPEG).
pub async fn snapshot(state: &Shared, printer: &Printer) -> anyhow::Result<Arc<Vec<u8>>> {
    let key = format!("{}|{}|{}|{}", printer.id, printer.camera, printer.camera_entity.as_deref().unwrap_or(""), printer.host);
    if let Some((at, img)) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).get(&key).cloned()
        && now() - at < FRESH_SECS
    {
        return Ok(img);
    }
    let img = Arc::new(match printer.camera.as_str() {
        "ha" => from_home_assistant(state, printer).await?,
        "printer" => from_printer(printer).await?,
        _ => bail!("no camera is set up for {}", printer.name),
    });
    CACHE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(key, (now(), img.clone()));
    Ok(img)
}

async fn from_home_assistant(state: &Shared, printer: &Printer) -> anyhow::Result<Vec<u8>> {
    let entity = printer.camera_entity.as_deref().filter(|e| crate::notify::is_entity_id(e, "camera")).ok_or_else(|| anyhow!("choose the Home Assistant camera"))?;
    let settings = state.store.read(|d| d.notify.clone()).await;
    let (base, token) = crate::notify::endpoint(&settings)?;
    let resp = state
        .http
        .get(format!("{base}/camera_proxy/{entity}"))
        .bearer_auth(token)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .context("couldn't reach Home Assistant")?;
    if !resp.status().is_success() {
        bail!("Home Assistant answered {} for {entity}", resp.status());
    }
    let bytes = resp.bytes().await?.to_vec();
    // Only ever hand a JPEG back, whatever Home Assistant answered with.
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        bail!("Home Assistant didn't send a JPEG for {entity}");
    }
    Ok(bytes)
}

/// The P1/A1 camera: TLS on port 6000, an 80-byte login, then frames of a
/// 16-byte header (little-endian payload size first) and a JPEG.
async fn from_printer(printer: &Printer) -> anyhow::Result<Vec<u8>> {
    if printer.printer_model.to_uppercase().contains(" X1") {
        bail!("the X1 camera streams over RTSP, which AirSlice can't read; use a Home Assistant camera instead");
    }
    let host = printer.host.as_str();
    let tcp = timeout(Duration::from_secs(5), TcpStream::connect((host, 6000)))
        .await
        .map_err(|_| anyhow!("the camera at {host}:6000 didn't answer"))?
        .with_context(|| format!("couldn't reach the camera at {host}:6000"))?;
    let connector = TlsConnector::from(Arc::new(printer_client_config()));
    let name = match host.parse::<std::net::IpAddr>() {
        Ok(ip) => rustls::pki_types::ServerName::IpAddress(ip.into()),
        Err(_) => rustls::pki_types::ServerName::try_from(host.to_string())?,
    };
    let mut tls = timeout(Duration::from_secs(5), connector.connect(name, tcp)).await.map_err(|_| anyhow!("camera TLS timed out"))??;
    tls.write_all(&login_packet(&printer.access_code)).await?;
    tls.flush().await?;
    let mut header = [0u8; 16];
    timeout(Duration::from_secs(10), tls.read_exact(&mut header))
        .await
        .map_err(|_| anyhow!("the camera sent no picture"))?
        .context("the camera closed the connection (check the access code)")?;
    let size = u32::from_le_bytes([header[0], header[1], header[2], header[3]]) as usize;
    if size == 0 || size > 8 * 1024 * 1024 {
        bail!("the camera sent a picture of unexpected size ({size} bytes)");
    }
    let mut img = vec![0u8; size];
    timeout(Duration::from_secs(10), tls.read_exact(&mut img)).await.map_err(|_| anyhow!("the camera picture didn't arrive"))??;
    if !img.starts_with(&[0xFF, 0xD8]) || !img.ends_with(&[0xFF, 0xD9]) {
        bail!("the camera sent something that isn't a JPEG");
    }
    Ok(img)
}

fn login_packet(access_code: &str) -> Vec<u8> {
    let mut p = Vec::with_capacity(80);
    for v in [0x40u32, 0x3000, 0, 0] {
        p.extend_from_slice(&v.to_le_bytes());
    }
    for field in ["bblp", access_code] {
        let mut b = field.as_bytes().to_vec();
        b.resize(32, 0);
        p.extend_from_slice(&b[..32]);
    }
    p
}

/// Keep a picture of the finished print with the job (best effort).
pub async fn save_photo(state: &Shared, printer: &Printer, job_id: &str) -> bool {
    if printer.camera.is_empty() {
        return false;
    }
    match snapshot(state, printer).await {
        Ok(img) => {
            let dir = state.cfg.data_dir.join("photos");
            let _ = tokio::fs::create_dir_all(&dir).await;
            tokio::fs::write(dir.join(format!("{job_id}.jpg")), img.as_slice()).await.is_ok()
        }
        Err(e) => {
            tracing::warn!("no photo of the finished print: {e:#}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_login() {
        let p = login_packet("12345678");
        assert_eq!(p.len(), 80);
        assert_eq!(&p[..8], &[0x40, 0, 0, 0, 0, 0x30, 0, 0]);
        assert_eq!(&p[16..20], b"bblp");
        assert_eq!(&p[48..56], b"12345678");
        assert_eq!(p[56], 0);
    }
}
