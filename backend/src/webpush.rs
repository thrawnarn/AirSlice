//! Browser notifications (Web Push): a browser that turned notifications on
//! gives an endpoint at its push service (Apple, Google, Mozilla) and keys.
//! Each message is encrypted for that browser (RFC 8291, aes128gcm) and the
//! request signed with AirSlice's own VAPID key (RFC 8292), kept in
//! state.json. The service worker (`frontend/public/sw.js`) shows it.

use crate::{
    models::{NotifyTarget, now},
    notify::Picture,
    state::Shared,
};
use anyhow::{Context, anyhow, bail};
use base64::Engine;
use ring::{
    aead, agreement, hkdf,
    rand::{SecureRandom, SystemRandom},
    signature::{self, EcdsaKeyPair, KeyPair},
};
use serde_json::json;
use std::sync::Mutex;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;
/// Who to contact about these pushes; Apple wants a mailto: or https URL.
const SUBJECT: &str = "https://github.com/thrawnarn/AirSlice";
/// Record size; a payload must stay below it (minus tag and delimiter).
const RECORD: u32 = 4096;
/// Pictures stay fetchable this long after their notification.
const PICTURE_SECS: i64 = 3600;

pub fn b64_decode(s: &str) -> anyhow::Result<Vec<u8>> {
    B64.decode(s.trim().trim_end_matches('=').replace('+', "-").replace('/', "_")).map_err(|_| anyhow!("not base64"))
}

/// AirSlice's signing key, made on first use.
async fn vapid(state: &Shared) -> anyhow::Result<EcdsaKeyPair> {
    let alg = &signature::ECDSA_P256_SHA256_FIXED_SIGNING;
    let rng = SystemRandom::new();
    let stored = match state.store.read(|d| d.webpush_key.clone()).await {
        Some(k) => k,
        None => {
            let doc = EcdsaKeyPair::generate_pkcs8(alg, &rng).map_err(|_| anyhow!("couldn't make a push key"))?;
            let k = B64.encode(doc.as_ref());
            // Two first uses at once keep the same key.
            state.store.update(move |d| d.webpush_key.get_or_insert(k).clone()).await?
        }
    };
    EcdsaKeyPair::from_pkcs8(alg, &b64_decode(&stored)?, &rng).map_err(|_| anyhow!("the stored push key is broken"))
}

/// The key browsers subscribe with (`applicationServerKey`), base64url.
pub async fn public_key(state: &Shared) -> anyhow::Result<String> {
    Ok(B64.encode(vapid(state).await?.public_key().as_ref()))
}

struct Len(usize);

impl hkdf::KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

fn hkdf(salt: &[u8], ikm: &[u8], info: &[u8], len: usize) -> Vec<u8> {
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, salt).extract(ikm);
    let mut out = vec![0; len];
    prk.expand(&[info], Len(len)).and_then(|okm| okm.fill(&mut out)).expect("HKDF lengths are fixed and small");
    out
}

/// Encrypt a payload for one browser: one aes128gcm record (RFC 8188) with
/// our one-time public key in the header.
pub fn encrypt(ua_public: &[u8], auth: &[u8], payload: &[u8]) -> anyhow::Result<Vec<u8>> {
    if ua_public.len() != 65 || auth.len() != 16 {
        bail!("the browser's keys look wrong; turn notifications on again from it");
    }
    if payload.len() + 17 > RECORD as usize {
        bail!("the notification is too long");
    }
    let rng = SystemRandom::new();
    let ours = agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng).map_err(|_| anyhow!("no random numbers"))?;
    let as_public = ours.compute_public_key().map_err(|_| anyhow!("no public key"))?.as_ref().to_vec();
    let secret = agreement::agree_ephemeral(ours, &agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, ua_public), |s| s.to_vec())
        .map_err(|_| anyhow!("the browser's key isn't valid"))?;
    let mut info = b"WebPush: info\0".to_vec();
    info.extend_from_slice(ua_public);
    info.extend_from_slice(&as_public);
    let ikm = hkdf(auth, &secret, &info, 32);
    let mut salt = [0u8; 16];
    rng.fill(&mut salt).map_err(|_| anyhow!("no random numbers"))?;
    let cek = hkdf(&salt, &ikm, b"Content-Encoding: aes128gcm\0", 16);
    let nonce = hkdf(&salt, &ikm, b"Content-Encoding: nonce\0", 12);
    let key = aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_128_GCM, &cek).map_err(|_| anyhow!("bad key"))?);
    let mut record = payload.to_vec();
    // The last (and only) record ends with 0x02 and no padding.
    record.push(2);
    key.seal_in_place_append_tag(aead::Nonce::try_assume_unique_for_key(&nonce).map_err(|_| anyhow!("bad nonce"))?, aead::Aad::empty(), &mut record)
        .map_err(|_| anyhow!("couldn't encrypt"))?;
    let mut out = Vec::with_capacity(21 + as_public.len() + record.len());
    out.extend_from_slice(&salt);
    out.extend_from_slice(&RECORD.to_be_bytes());
    out.push(as_public.len() as u8);
    out.extend_from_slice(&as_public);
    out.extend_from_slice(&record);
    Ok(out)
}

/// The signed token a push service checks: for the endpoint's origin, 12 h.
fn jwt(key: &EcdsaKeyPair, endpoint: &str) -> anyhow::Result<String> {
    let aud = url::Url::parse(endpoint)?.origin().ascii_serialization();
    let header = B64.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
    let claims = B64.encode(json!({ "aud": aud, "exp": now() + 12 * 3600, "sub": SUBJECT }).to_string());
    let input = format!("{header}.{claims}");
    let sig = key.sign(&SystemRandom::new(), input.as_bytes()).map_err(|_| anyhow!("couldn't sign"))?;
    Ok(format!("{input}.{}", B64.encode(sig.as_ref())))
}

/// A push endpoint we'll post to: https, a host, not too long.
pub fn is_endpoint(s: &str) -> bool {
    s.len() <= 2048 && url::Url::parse(s).is_ok_and(|u| u.scheme() == "https" && u.host_str().is_some())
}

/// Send one notification to a browser. A browser that unsubscribed (404,
/// 410) is turned off, so it isn't tried again.
pub async fn send(state: &Shared, t: &NotifyTarget, title: &str, message: &str, job: Option<&str>, photo: Option<&Picture>) -> anyhow::Result<()> {
    let endpoint = t.push_endpoint.trim();
    if !is_endpoint(endpoint) {
        bail!("turn notifications on from that browser first");
    }
    let mut payload = json!({
        "title": title,
        "body": message.chars().take(1000).collect::<String>(),
        // Relative: the service worker resolves them against its own address.
        "url": job.map(|j| format!("./#/job/{j}")).unwrap_or_else(|| "./".into()),
    });
    if let Some(p) = photo {
        payload["image"] = json!(format!("api/notify/picture/{}", keep_picture(p)));
    }
    let body = encrypt(&b64_decode(&t.push_p256dh)?, &b64_decode(&t.push_auth)?, payload.to_string().as_bytes())?;
    let key = vapid(state).await?;
    let resp = state
        .http
        .post(endpoint)
        .header("TTL", "86400")
        .header("Urgency", "normal")
        .header("Content-Encoding", "aes128gcm")
        .header("Content-Type", "application/octet-stream")
        .header("Authorization", format!("vapid t={}, k={}", jwt(&key, endpoint)?, B64.encode(key.public_key().as_ref())))
        .body(body)
        .send()
        .await
        .context("couldn't reach the browser's push service")?;
    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }
    if matches!(status.as_u16(), 404 | 410) {
        let id = t.id.clone();
        let _ = state
            .store
            .update(move |d| {
                if let Some(t) = d.notify.targets.iter_mut().find(|t| t.id == id) {
                    t.enabled = false;
                }
            })
            .await;
        bail!("this browser no longer takes notifications (turned off here); turn them on again from it");
    }
    let text = resp.text().await.unwrap_or_default();
    bail!("the push service answered {status}: {}", text.chars().take(200).collect::<String>())
}

/// Pictures for notifications, fetched by the browser at an unguessable
/// address for an hour (it can't send AirSlice's login with that request).
static PICTURES: Mutex<Vec<(String, i64, Picture)>> = Mutex::new(Vec::new());

fn keep_picture(p: &Picture) -> String {
    let id = crate::auth::random_token().chars().filter(|c| c.is_ascii_alphanumeric()).take(32).collect::<String>();
    let mut g = PICTURES.lock().unwrap_or_else(|e| e.into_inner());
    g.retain(|(_, at, _)| now() - at < PICTURE_SECS);
    if g.len() >= 20 {
        g.remove(0);
    }
    g.push((id.clone(), now(), p.clone()));
    id
}

/// A notification picture by id: (bytes, content type).
pub fn picture(id: &str) -> Option<(std::sync::Arc<Vec<u8>>, &'static str)> {
    let g = PICTURES.lock().unwrap_or_else(|e| e.into_inner());
    g.iter().find(|(i, at, _)| i == id && now() - at < PICTURE_SECS).map(|(_, _, p)| (p.bytes.clone(), p.mime()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// What a browser does with the message: undo `encrypt` with its own key.
    fn decrypt(ua: agreement::EphemeralPrivateKey, ua_public: &[u8], auth: &[u8], body: &[u8]) -> Vec<u8> {
        let (salt, rest) = body.split_at(16);
        assert_eq!(u32::from_be_bytes(rest[..4].try_into().unwrap()), RECORD);
        let idlen = rest[4] as usize;
        let (as_public, record) = rest[5..].split_at(idlen);
        let secret = agreement::agree_ephemeral(ua, &agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, as_public), |s| s.to_vec()).unwrap();
        let mut info = b"WebPush: info\0".to_vec();
        info.extend_from_slice(ua_public);
        info.extend_from_slice(as_public);
        let ikm = hkdf(auth, &secret, &info, 32);
        let cek = hkdf(salt, &ikm, b"Content-Encoding: aes128gcm\0", 16);
        let nonce = hkdf(salt, &ikm, b"Content-Encoding: nonce\0", 12);
        let key = aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_128_GCM, &cek).unwrap());
        let mut buf = record.to_vec();
        let plain = key.open_in_place(aead::Nonce::try_assume_unique_for_key(&nonce).unwrap(), aead::Aad::empty(), &mut buf).unwrap();
        assert_eq!(plain.last(), Some(&2));
        plain[..plain.len() - 1].to_vec()
    }

    #[test]
    fn a_browser_can_read_it() {
        let rng = SystemRandom::new();
        let ua = agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng).unwrap();
        let ua_public = ua.compute_public_key().unwrap().as_ref().to_vec();
        let auth = [7u8; 16];
        let body = encrypt(&ua_public, &auth, "Printed on Workshop — ”benchy”".as_bytes()).unwrap();
        assert_eq!(decrypt(ua, &ua_public, &auth, &body), "Printed on Workshop — ”benchy”".as_bytes());
        assert!(encrypt(&ua_public[..64], &auth, b"x").is_err());
    }

    #[test]
    fn the_token_verifies() {
        let rng = SystemRandom::new();
        let alg = &signature::ECDSA_P256_SHA256_FIXED_SIGNING;
        let key = EcdsaKeyPair::from_pkcs8(alg, EcdsaKeyPair::generate_pkcs8(alg, &rng).unwrap().as_ref(), &rng).unwrap();
        let token = jwt(&key, "https://web.push.apple.com/QGuQyavXutnMH").unwrap();
        let (input, sig) = token.rsplit_once('.').unwrap();
        signature::UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_FIXED, key.public_key().as_ref())
            .verify(input.as_bytes(), &b64_decode(sig).unwrap())
            .unwrap();
        let claims: Value = serde_json::from_slice(&b64_decode(input.split('.').nth(1).unwrap()).unwrap()).unwrap();
        assert_eq!(claims["aud"], "https://web.push.apple.com");
        assert!(is_endpoint("https://fcm.googleapis.com/fcm/send/abc"));
        assert!(!is_endpoint("http://10.0.0.1/x"));
        assert!(!is_endpoint("file:///etc/passwd"));
    }
}
