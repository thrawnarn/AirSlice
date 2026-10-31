//! Minimal implicit-FTPS client (port 990) for uploading jobs to the printer.
//!
//! Quirks handled here:
//! * X1/P1/P2/H2 run vsftpd, which requires the data channel to be TLS with
//!   the *same* session as the control channel (session resumption).
//! * A1 / A1 mini stall when the data channel is encrypted, so they get a
//!   plain data channel (PROT C) while login stays encrypted.

use crate::models::FtpDataMode;
use crate::tls::printer_client_config;
use anyhow::{Context, anyhow, bail};
use rustls::pki_types::ServerName;
use std::{net::IpAddr, path::Path, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    net::TcpStream,
    time::timeout,
};
use tokio_rustls::{TlsConnector, client::TlsStream};

const PORT: u16 = 990;
const CMD_TIMEOUT: Duration = Duration::from_secs(20);
/// After the last byte, slow printers (A1 on weak Wi-Fi) can take a while to
/// confirm the transfer.
const FINISH_TIMEOUT: Duration = Duration::from_secs(120);

pub struct FtpTarget<'a> {
    pub host: &'a str,
    pub access_code: &'a str,
}

struct Control {
    stream: BufReader<TlsStream<TcpStream>>,
}

#[derive(Debug)]
struct Reply {
    code: u16,
    text: String,
}

impl Control {
    async fn read_reply(&mut self, wait: Duration) -> anyhow::Result<Reply> {
        timeout(wait, async {
            let mut line = String::new();
            if self.stream.read_line(&mut line).await? == 0 {
                bail!("printer closed the FTP connection");
            }
            let code: u16 = line.get(0..3).and_then(|c| c.parse().ok()).ok_or_else(|| anyhow!("bad FTP reply: {line:?}"))?;
            let mut text = line.trim_end().to_string();
            // Multi-line reply: "123-..." until "123 ..."
            if line.as_bytes().get(3) == Some(&b'-') {
                let end = format!("{code} ");
                loop {
                    line.clear();
                    if self.stream.read_line(&mut line).await? == 0 {
                        bail!("printer closed the FTP connection");
                    }
                    text.push('\n');
                    text.push_str(line.trim_end());
                    if line.starts_with(&end) {
                        break;
                    }
                }
            }
            Ok(Reply { code, text })
        })
        .await
        .map_err(|_| anyhow!("timed out waiting for the printer's FTP server"))?
    }

    async fn cmd(&mut self, cmd: &str) -> anyhow::Result<Reply> {
        let shown = if cmd.starts_with("PASS ") { "PASS ****" } else { cmd };
        tracing::debug!("FTP > {shown}");
        self.stream.get_mut().write_all(format!("{cmd}\r\n").as_bytes()).await?;
        self.stream.get_mut().flush().await?;
        let reply = self.read_reply(CMD_TIMEOUT).await?;
        tracing::debug!("FTP < {}", reply.text);
        Ok(reply)
    }

    async fn expect(&mut self, cmd: &str, ok: &[u16]) -> anyhow::Result<Reply> {
        let reply = self.cmd(cmd).await?;
        if !ok.contains(&reply.code) {
            let shown = cmd.split_whitespace().next().unwrap_or(cmd);
            bail!("printer refused {shown}: {}", reply.text);
        }
        Ok(reply)
    }
}

fn server_name(host: &str) -> anyhow::Result<ServerName<'static>> {
    Ok(match host.parse::<IpAddr>() {
        Ok(ip) => ServerName::IpAddress(ip.into()),
        Err(_) => ServerName::try_from(host.to_string()).map_err(|_| anyhow!("invalid printer host {host:?}"))?,
    })
}

async fn login(target: &FtpTarget<'_>, connector: &TlsConnector) -> anyhow::Result<Control> {
    let tcp = timeout(CMD_TIMEOUT, TcpStream::connect((target.host, PORT)))
        .await
        .map_err(|_| anyhow!("could not reach {}:{PORT} (timeout)", target.host))?
        .with_context(|| format!("could not reach {}:{PORT}", target.host))?;
    let tls = timeout(CMD_TIMEOUT, connector.connect(server_name(target.host)?, tcp))
        .await
        .map_err(|_| anyhow!("TLS handshake with the printer timed out"))?
        .context("TLS handshake with the printer failed")?;
    let mut ctl = Control { stream: BufReader::new(tls) };
    let greeting = ctl.read_reply(CMD_TIMEOUT).await?;
    if greeting.code != 220 {
        bail!("unexpected FTP greeting: {}", greeting.text);
    }
    ctl.expect("USER bblp", &[331, 230]).await?;
    let pass = ctl.cmd(&format!("PASS {}", target.access_code)).await?;
    if pass.code != 230 {
        bail!("printer rejected the access code ({})", pass.text);
    }
    ctl.expect("PBSZ 0", &[200]).await?;
    Ok(ctl)
}

fn parse_pasv(text: &str) -> anyhow::Result<u16> {
    let start = text.find('(').ok_or_else(|| anyhow!("bad PASV reply: {text}"))?;
    let end = text[start..].find(')').ok_or_else(|| anyhow!("bad PASV reply: {text}"))? + start;
    // Each number is a byte (u8 refuses 300), so the port can't overflow.
    let nums: Vec<u8> = text[start + 1..end].split(',').filter_map(|n| n.trim().parse().ok()).collect();
    if nums.len() != 6 {
        bail!("bad PASV reply: {text}");
    }
    Ok(u16::from(nums[4]) << 8 | u16::from(nums[5]))
}

/// Check that we can log in. Used by "Test connection".
pub async fn check_login(target: &FtpTarget<'_>) -> anyhow::Result<()> {
    let connector = TlsConnector::from(Arc::new(printer_client_config()));
    let mut ctl = login(target, &connector).await?;
    let _ = ctl.cmd("QUIT").await;
    Ok(())
}

/// Size of a file in the printer's storage root, `None` when it isn't there.
pub async fn remote_size(target: &FtpTarget<'_>, remote_name: &str) -> anyhow::Result<Option<u64>> {
    let connector = TlsConnector::from(Arc::new(printer_client_config()));
    let mut ctl = login(target, &connector).await?;
    ctl.expect("TYPE I", &[200]).await?;
    let reply = ctl.cmd(&format!("SIZE {remote_name}")).await?;
    let _ = ctl.cmd("QUIT").await;
    Ok(size_reply(reply.code, &reply.text))
}

/// "213 1234567" → 1234567; anything else (550 not found) → None.
fn size_reply(code: u16, text: &str) -> Option<u64> {
    (code == 213).then(|| text.split_whitespace().nth(1)?.parse().ok()).flatten()
}

/// Upload `local` to the printer's storage root as `remote_name`.
/// Called with (bytes sent, total bytes) while uploading.
pub type Progress<'a> = Option<&'a (dyn Fn(u64, u64) + Send + Sync)>;

pub async fn upload(
    target: &FtpTarget<'_>,
    mode: FtpDataMode,
    a1_family: bool,
    local: &Path,
    remote_name: &str,
    progress: Progress<'_>,
) -> anyhow::Result<()> {
    let encrypted_first = match mode {
        FtpDataMode::Private => true,
        FtpDataMode::Clear => false,
        FtpDataMode::Auto => !a1_family,
    };
    match upload_once(target, encrypted_first, local, remote_name, progress).await {
        Ok(()) => Ok(()),
        Err(first) if mode == FtpDataMode::Auto => {
            tracing::warn!("FTPS upload failed ({first:#}); retrying with the other data-channel mode");
            upload_once(target, !encrypted_first, local, remote_name, progress)
                .await
                .with_context(|| format!("both FTPS modes failed; first attempt: {first:#}"))
        }
        Err(e) => Err(e),
    }
}

async fn upload_once(target: &FtpTarget<'_>, encrypted: bool, local: &Path, remote_name: &str, progress: Progress<'_>) -> anyhow::Result<()> {
    // One config per upload so the control session is the one the data
    // channel resumes.
    let connector = TlsConnector::from(Arc::new(printer_client_config()));
    let mut ctl = login(target, &connector).await?;
    ctl.expect(if encrypted { "PROT P" } else { "PROT C" }, &[200]).await?;
    ctl.expect("TYPE I", &[200]).await?;
    let pasv = ctl.expect("PASV", &[227]).await?;
    // Ignore the address in the reply; printers sometimes advertise an
    // address that is not reachable from containers.
    let port = parse_pasv(&pasv.text)?;
    let data_tcp = timeout(CMD_TIMEOUT, TcpStream::connect((target.host, port)))
        .await
        .map_err(|_| anyhow!("data connection to the printer timed out"))??;
    let stor = ctl.cmd(&format!("STOR {remote_name}")).await?;
    if stor.code != 150 && stor.code != 125 {
        bail!("printer refused the upload: {}", stor.text);
    }

    let mut file = tokio::fs::File::open(local).await.with_context(|| format!("opening {}", local.display()))?;
    if encrypted {
        let mut data = timeout(CMD_TIMEOUT, connector.connect(server_name(target.host)?, data_tcp))
            .await
            .map_err(|_| anyhow!("TLS handshake on the data channel timed out"))?
            .context("TLS handshake on the data channel failed")?;
        copy_and_close(&mut file, &mut data, progress).await?;
    } else {
        let mut data = data_tcp;
        copy_and_close(&mut file, &mut data, progress).await?;
    }

    let done = ctl.read_reply(FINISH_TIMEOUT).await?;
    if done.code != 226 && done.code != 250 {
        bail!("printer did not confirm the upload: {}", done.text);
    }
    let _ = ctl.cmd("QUIT").await;
    Ok(())
}

async fn copy_and_close<W>(src: &mut tokio::fs::File, dst: &mut W, progress: Progress<'_>) -> anyhow::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let total = src.metadata().await.map(|m| m.len()).unwrap_or(0);
    let copy = async {
        let mut buf = vec![0u8; 256 * 1024];
        let mut sent = 0u64;
        loop {
            let n = src.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            dst.write_all(&buf[..n]).await?;
            sent += n as u64;
            if let Some(p) = progress {
                p(sent, total);
            }
        }
        Ok::<_, std::io::Error>(())
    };
    timeout(Duration::from_secs(30 * 60), copy)
        .await
        .map_err(|_| anyhow!("upload took too long"))?
        .context("upload interrupted")?;
    dst.flush().await?;
    // For TLS this sends close_notify, which vsftpd requires before it
    // accepts the transfer as complete.
    dst.shutdown().await.context("closing data connection")?;
    Ok(())
}

/// A file name the printer firmware handles well: ASCII only, no spaces.
pub fn safe_remote_name(title: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('_');
            last_dash = true;
        }
        if out.len() >= 48 {
            break;
        }
    }
    let out = out.trim_end_matches('_');
    let base = if out.is_empty() { "print" } else { out };
    format!("{base}.3mf")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasv_port_in_range_only() {
        assert_eq!(parse_pasv("227 Entering Passive Mode (10,99,50,20,195,80)").unwrap(), 195 * 256 + 80);
        assert!(parse_pasv("227 (1,2,3,4,300,1)").is_err());
        assert!(parse_pasv("227 (1,2,3,4,5)").is_err());
    }

    #[test]
    fn reads_size_replies() {
        assert_eq!(size_reply(213, "213 7340032"), Some(7_340_032));
        assert_eq!(size_reply(550, "550 Could not get file size."), None);
    }

    #[test]
    fn pasv_port() {
        assert_eq!(parse_pasv("227 Entering Passive Mode (192,168,1,20,195,80).").unwrap(), 195 * 256 + 80);
        assert!(parse_pasv("227 nope").is_err());
    }

    #[test]
    fn remote_names() {
        assert_eq!(safe_remote_name("Benchy – the 3D torture test!"), "Benchy_the_3D_torture_test.3mf");
        assert_eq!(safe_remote_name("åäö"), "print.3mf");
        assert!(safe_remote_name(&"x".repeat(200)).len() <= 52);
    }
}
