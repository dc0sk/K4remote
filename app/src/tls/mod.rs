//! Encrypted connections for the spot networks (FR-SPOT-13), with an option to **approve an
//! untrusted certificate by hand**.
//!
//! A connection is verified the ordinary way first, against the public certificate authorities in
//! `webpki-roots`. If that fails for a reason about the *certificate* (an unknown issuer — a
//! self-signed server, an expired or mismatched certificate), the connection is refused and the
//! certificate's SHA-256 fingerprint and the reason are handed back as
//! [`ConnectError::Untrusted`], so the operator can look at them and decide. An approved
//! certificate is kept as a [`Pin`] and accepted on later connections. Three rules keep that from
//! being a hole:
//!
//! * **An approval is exact.** It names one host, one port and one certificate. A pin for another
//!   port, another host, or a *different* certificate on the same one does not match; a different
//!   certificate where one was approved is reported as `changed`, which is more suspicious than a
//!   first sight and is worded that way.
//! * **A fingerprint is not a password.** The certificate is public, so anyone can present it. An
//!   approved certificate is accepted only if the server also proves it holds the *private key*: the
//!   handshake signature is still verified against the certificate's key.
//! * **Only problems with the certificate can be approved.** A protocol error, a bad signature or
//!   a refused version is never turned into an approval prompt.
//!
//! An approval also overrides expiry and a name mismatch — that is what approving an untrusted
//! certificate means, and the reason is shown *before* the operator decides.

#[cfg(test)]
pub(crate) mod test_certs;

use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use k4_spot::mqtt_source::{CertInfo, ConnectError, Connector, Wire};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, ClientConnection, DigitallySignedStruct, Error, RootCertStore,
    SignatureScheme, StreamOwned,
};
use sha2::{Digest, Sha256};

/// One approved certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    pub host: String,
    pub port: u16,
    /// SHA-256 of the certificate, 64 lower-case hex digits.
    pub sha256: String,
}

/// The approved certificates, shared with the connector so an approval takes effect on the next
/// connection without restarting anything.
pub type Pins = Arc<Mutex<Vec<Pin>>>;

/// The pins for the approved certificates in the configuration, malformed entries already dropped.
pub fn pins_from(certs: &[k4_config::TrustedCert]) -> Vec<Pin> {
    certs
        .iter()
        .filter(|c| c.is_valid())
        .map(|c| Pin {
            host: c.host.clone(),
            port: c.port,
            sha256: c.sha256.clone(),
        })
        .collect()
}

/// The SHA-256 of a certificate as sent, as 64 lower-case hex digits.
pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The fingerprint in the form people compare — upper case, colon-separated pairs, as `openssl`
/// and browsers show it.
pub fn display_fingerprint(sha256: &str) -> String {
    sha256
        .as_bytes()
        .chunks(2)
        .map(|c| String::from_utf8_lossy(c).to_uppercase())
        .collect::<Vec<_>>()
        .join(":")
}

/// Why a certificate was refused, in a few words that read inside parentheses.
fn describe(e: &Error) -> String {
    let Error::InvalidCertificate(c) = e else {
        return e.to_string();
    };
    match c {
        CertificateError::UnknownIssuer => {
            "unknown issuer — not signed by a known authority".to_string()
        }
        CertificateError::Expired | CertificateError::ExpiredContext { .. } => "expired".into(),
        CertificateError::NotValidYet | CertificateError::NotValidYetContext { .. } => {
            "not valid yet".into()
        }
        CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. } => {
            "its name does not match the server".into()
        }
        CertificateError::BadSignature => "its signature is bad".into(),
        CertificateError::Revoked => "revoked".into(),
        // A certificate a server made for itself is very often marked as an authority (`openssl
        // req -x509` does so by default), and is then refused for that before its issuer is even
        // looked at. The library gives no variant for it, only the text.
        CertificateError::Other(o) if format!("{o:?}").contains("CaUsedAsEndEntity") => {
            "self-signed — an authority's own certificate used as the server's".into()
        }
        other => format!("not acceptable: {other:?}"),
    }
}

/// Verifies as usual, and accepts an approved certificate the ordinary check refused.
#[derive(Debug)]
struct PinVerifier {
    /// The ordinary check (an injectable trait object so a test can make it fail in ways the real
    /// one cannot be made to with a certificate).
    inner: Arc<dyn ServerCertVerifier>,
    provider: Arc<CryptoProvider>,
    host: String,
    port: u16,
    pins: Pins,
    /// What was refused on this connection, for the caller to hand to the operator.
    seen: Arc<Mutex<Option<CertInfo>>>,
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let refused = match self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Ok(ok) => return Ok(ok),
            Err(e) => e,
        };
        // Only a problem with the certificate can be approved.
        if !matches!(refused, Error::InvalidCertificate(_)) {
            return Err(refused);
        }
        let fp = fingerprint(end_entity.as_ref());
        let approved: Vec<String> = self
            .pins
            .lock()
            .map(|p| {
                p.iter()
                    .filter(|p| p.host.eq_ignore_ascii_case(&self.host) && p.port == self.port)
                    .map(|p| p.sha256.clone())
                    .collect()
            })
            .unwrap_or_default();
        if approved.contains(&fp) {
            return Ok(ServerCertVerified::assertion());
        }
        if let Ok(mut seen) = self.seen.lock() {
            *seen = Some(CertInfo {
                host: self.host.clone(),
                port: self.port,
                sha256: fp,
                reason: describe(&refused),
                changed: !approved.is_empty(),
            });
        }
        Err(refused)
    }

    // The private-key proof is checked even for an approved certificate: the certificate itself
    // is public and proves nothing.
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// How long the handshake may take, at most, on each step.
const HANDSHAKE_STEP: Duration = Duration::from_secs(10);

/// A connector that makes TLS connections, consulting `pins` for approved certificates.
pub fn connector(pins: Pins) -> Connector {
    Arc::new(move |host, port, timeout| connect(host, port, timeout, &pins))
}

fn connect(
    host: &str,
    port: u16,
    timeout: Duration,
    pins: &Pins,
) -> Result<Box<dyn Wire>, ConnectError> {
    let failed = |why: String| ConnectError::Failed(why);
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let roots = Arc::new(RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    });
    let inner: Arc<dyn ServerCertVerifier> =
        WebPkiServerVerifier::builder_with_provider(roots, Arc::clone(&provider))
            .build()
            .map_err(|e| failed(format!("cannot set up certificate checking: {e}")))?;
    let seen = Arc::new(Mutex::new(None));
    let verifier = Arc::new(PinVerifier {
        inner,
        provider: Arc::clone(&provider),
        host: host.to_string(),
        port,
        pins: Arc::clone(pins),
        seen: Arc::clone(&seen),
    });
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| failed(format!("cannot set up TLS: {e}")))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    let name = ServerName::try_from(host.to_string())
        .map_err(|_| failed(format!("{host} is not a valid server name")))?;

    // Bounded (FR-SPOT-14), as the plain connectors are: resolution has no timeout of its own, and
    // this runs on the thread every spot network is polled from.
    let addrs = k4_spot::dns::resolve_bounded(host, port, timeout)
        .map_err(|e| failed(format!("cannot resolve {host}: {e}")))?;
    let mut last = None;
    let mut tcp = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(s) => {
                tcp = Some(s);
                break;
            }
            Err(e) => last = Some(e),
        }
    }
    let mut tcp = tcp.ok_or_else(|| {
        failed(format!(
            "connect to {host}:{port} failed: {}",
            last.map_or_else(|| "no address to connect to".to_string(), |e| e.to_string())
        ))
    })?;
    let _ = tcp.set_nodelay(true);
    let _ = tcp.set_read_timeout(Some(HANDSHAKE_STEP));
    let _ = tcp.set_write_timeout(Some(HANDSHAKE_STEP));

    let mut conn = ClientConnection::new(Arc::new(config), name)
        .map_err(|e| failed(format!("cannot start TLS: {e}")))?;
    // Run the handshake to the end here, so a refused certificate is reported now and not on the
    // first read.
    while conn.is_handshaking() {
        if let Err(e) = conn.complete_io(&mut tcp) {
            if let Some(info) = seen.lock().ok().and_then(|mut s| s.take()) {
                return Err(ConnectError::Untrusted(info));
            }
            return Err(failed(format!(
                "secure connection to {host}:{port} failed: {e}"
            )));
        }
    }
    // From here on a read gives up after a short while, like a plain connection's does.
    tcp.set_read_timeout(Some(Duration::from_millis(20)))
        .map_err(|e| failed(format!("cannot set a read timeout: {e}")))?;
    let _ = tcp.set_write_timeout(Some(Duration::from_secs(5)));
    Ok(Box::new(StreamOwned::new(conn, tcp)))
}

/// A TLS server for tests, shared with the worker's tests.
#[cfg(test)]
pub(crate) mod testkit {
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use rustls::server::{ClientHello, ResolvesServerCert};
    use rustls::sign::CertifiedKey;
    use rustls::{ServerConfig, ServerConnection, StreamOwned};

    /// A server certificate resolver that pairs any chain with any key — no consistency check, so a
    /// test can present one certificate while signing with another's key.
    #[derive(Debug)]
    struct Resolver(Arc<CertifiedKey>);

    impl ResolvesServerCert for Resolver {
        fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
            Some(Arc::clone(&self.0))
        }
    }

    /// What runs for each accepted connection.
    pub(crate) type Handler = Arc<dyn Fn(StreamOwned<ServerConnection, TcpStream>) + Send + Sync>;

    /// A TLS server on loopback presenting `cert` and signing with `key`, limited to `versions`.
    /// It accepts `accepts` connections, one after another, and runs `handler` on each (the
    /// handshake happens on its first read or write, so a client that refuses the certificate
    /// costs an accept). Returns the port.
    pub(crate) fn serve_tls(
        cert: &'static [u8],
        key: &'static [u8],
        versions: &'static [&'static rustls::SupportedProtocolVersion],
        accepts: usize,
        handler: Handler,
    ) -> u16 {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let signing = provider
            .key_provider
            .load_private_key(PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.to_vec())))
            .expect("test key");
        let certified = Arc::new(CertifiedKey::new(
            vec![CertificateDer::from(cert.to_vec())],
            signing,
        ));
        let config = Arc::new(
            ServerConfig::builder_with_provider(provider)
                .with_protocol_versions(versions)
                .unwrap()
                .with_no_client_auth()
                .with_cert_resolver(Arc::new(Resolver(certified))),
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            for _ in 0..accepts {
                let Ok((tcp, _)) = listener.accept() else {
                    return;
                };
                let _ = tcp.set_read_timeout(Some(Duration::from_secs(5)));
                let Ok(conn) = ServerConnection::new(Arc::clone(&config)) else {
                    continue;
                };
                let handler = Arc::clone(&handler);
                thread::spawn(move || handler(StreamOwned::new(conn, tcp)));
            }
        });
        port
    }
}

#[cfg(test)]
mod tests {
    use super::test_certs::*;
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::thread;

    /// A TLS echo server on loopback presenting `cert` and signing with `key`. It answers one
    /// connection: it reads what arrives and writes it back. Returns the port.
    fn serve(cert: &'static [u8], key: &'static [u8]) -> u16 {
        serve_versions(cert, key, rustls::DEFAULT_VERSIONS)
    }

    /// As [`serve`], limited to the given protocol versions.
    fn serve_versions(
        cert: &'static [u8],
        key: &'static [u8],
        versions: &'static [&'static rustls::SupportedProtocolVersion],
    ) -> u16 {
        testkit::serve_tls(
            cert,
            key,
            versions,
            1,
            Arc::new(|mut tls| {
                let mut buf = [0u8; 256];
                if let Ok(n) = tls.read(&mut buf) {
                    if n > 0 {
                        let _ = tls.write_all(&buf[..n]);
                        let _ = tls.flush();
                    }
                }
                thread::sleep(Duration::from_millis(150));
            }),
        )
    }

    fn pins(list: &[(&str, u16, &str)]) -> Pins {
        Arc::new(Mutex::new(
            list.iter()
                .map(|(h, p, f)| Pin {
                    host: (*h).into(),
                    port: *p,
                    sha256: (*f).into(),
                })
                .collect(),
        ))
    }

    fn connect_to(port: u16, pins: &Pins) -> Result<Box<dyn Wire>, ConnectError> {
        connect("127.0.0.1", port, Duration::from_secs(5), pins)
    }

    /// FR-SPOT-13: a certificate no known authority signed is refused, and the refusal carries the
    /// certificate's SHA-256 fingerprint — checked against the value `openssl` computed — and the
    /// reason, so the operator can decide.
    /// trace: FR-SPOT-13
    #[test]
    fn fr_spot_13_untrusted_certificate_is_refused_with_its_fingerprint() {
        let port = serve(A_CERT, A_KEY);
        let err = connect_to(port, &pins(&[])).err().expect("must be refused");
        let ConnectError::Untrusted(info) = err else {
            panic!("expected an untrusted certificate, got {err:?}");
        };
        assert_eq!(info.sha256, A_SHA256, "fingerprint differs from openssl's");
        assert_eq!(info.host, "127.0.0.1");
        assert_eq!(info.port, port);
        assert!(!info.changed, "a first sight is not a change");
        // `openssl req -x509` marks a certificate as an authority by default, so this is the
        // common case of a server that made its own certificate.
        assert!(info.reason.starts_with("self-signed"), "{}", info.reason);

        // A self-signed certificate not marked as an authority has an unknown issuer instead.
        let port = serve(C_CERT, C_KEY);
        let ConnectError::Untrusted(info) = connect_to(port, &pins(&[])).err().unwrap() else {
            panic!("expected an untrusted certificate");
        };
        assert_eq!(info.sha256, C_SHA256);
        assert!(info.reason.starts_with("unknown issuer"), "{}", info.reason);
        // Either can be approved and then connects (a fresh server: each answers one connection).
        let port = serve(C_CERT, C_KEY);
        assert!(connect_to(port, &pins(&[("127.0.0.1", port, C_SHA256)])).is_ok());
    }

    /// FR-SPOT-13: once approved, the same certificate connects, and the connection carries data
    /// both ways through the encryption.
    /// trace: FR-SPOT-13
    #[test]
    fn fr_spot_13_an_approved_certificate_connects_and_carries_data() {
        let port = serve(A_CERT, A_KEY);
        let mut wire = connect_to(port, &pins(&[("127.0.0.1", port, A_SHA256)]))
            .expect("an approved certificate must connect");
        wire.write_all(b"hello over tls").unwrap();
        wire.flush().unwrap();
        let mut got = Vec::new();
        let mut buf = [0u8; 64];
        let t0 = std::time::Instant::now();
        while got.len() < 14 && t0.elapsed() < Duration::from_secs(5) {
            match wire.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => got.extend_from_slice(&buf[..n]),
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(e) => panic!("read failed: {e}"),
            }
        }
        assert_eq!(got, b"hello over tls");
        // Once the data has been read, a further read gives up promptly instead of blocking: the
        // source polls with a bounded time and depends on it.
        let t0 = std::time::Instant::now();
        match wire.read(&mut buf) {
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            other => panic!("expected a timeout, got {other:?}"),
        }
        assert!(
            t0.elapsed() < Duration::from_millis(500),
            "{:?}",
            t0.elapsed()
        );
    }

    /// FR-SPOT-13: the same holds on TLS 1.2, which signs the handshake differently: an approved
    /// certificate connects, and a replayed one that cannot prove its key does not.
    /// trace: FR-SPOT-13
    #[test]
    fn fr_spot_13_tls12_also_checks_the_key() {
        // Leaked on purpose: a slice of references to a `static` is not promoted to `'static`.
        let tls12: &'static [&'static rustls::SupportedProtocolVersion] =
            Box::leak(Box::new([&rustls::version::TLS12]));
        let port = serve_versions(A_CERT, A_KEY, tls12);
        assert!(connect_to(port, &pins(&[("127.0.0.1", port, A_SHA256)])).is_ok());
        let port = serve_versions(A_CERT, B_KEY, tls12);
        match connect_to(port, &pins(&[("127.0.0.1", port, A_SHA256)])) {
            Err(ConnectError::Failed(why)) => assert!(why.contains("secure connection"), "{why}"),
            other => panic!(
                "a replayed certificate connected on TLS 1.2: {:?}",
                other.err()
            ),
        }
    }

    /// FR-SPOT-13: an approval is exact — another port, another host, a different certificate on
    /// the same host and port, or a fingerprint that is only nearly right does not match; a
    /// different certificate where one was approved is reported as changed.
    /// trace: FR-SPOT-13
    #[test]
    fn fr_spot_13_an_approval_is_exact() {
        let port = serve(A_CERT, A_KEY);
        let untrusted = |p: &Pins, port: u16| match connect_to(port, p) {
            Err(ConnectError::Untrusted(i)) => i,
            other => panic!("expected untrusted, got {:?}", other.err()),
        };
        // Approved for a different port: still untrusted, and not a "change" (nothing was
        // approved here).
        let i = untrusted(
            &pins(&[("127.0.0.1", port.wrapping_add(1), A_SHA256)]),
            port,
        );
        assert!(!i.changed);

        // Approved for a different host name.
        let port = serve(A_CERT, A_KEY);
        let i = untrusted(&pins(&[("localhost", port, A_SHA256)]), port);
        assert!(!i.changed);

        // A different certificate approved for this host and port: the server's has *changed*.
        let port = serve(A_CERT, A_KEY);
        let i = untrusted(&pins(&[("127.0.0.1", port, B_SHA256)]), port);
        assert!(i.changed, "an approved certificate was replaced by another");
        assert_eq!(i.sha256, A_SHA256);

        // Nearly right is wrong: a prefix, a suffix, upper case, an empty pin.
        for bad in [
            &A_SHA256[..63],
            &format!("{A_SHA256}0")[..],
            &A_SHA256.to_uppercase()[..],
            "",
        ] {
            let port = serve(A_CERT, A_KEY);
            let i = untrusted(&pins(&[("127.0.0.1", port, bad)]), port);
            assert_eq!(i.sha256, A_SHA256, "pin {bad:?}");
        }

        // The right pin among others is found.
        let port = serve(A_CERT, A_KEY);
        let p = pins(&[
            ("127.0.0.1", port, B_SHA256),
            ("127.0.0.1", port.wrapping_add(1), A_SHA256),
            ("127.0.0.1", port, A_SHA256),
        ]);
        assert!(connect_to(port, &p).is_ok());
        // The host name is not case sensitive: the server is reached as `localhost`, the approval
        // was recorded as `LocalHost`.
        let port = serve(A_CERT, A_KEY);
        let p = pins(&[("LocalHost", port, A_SHA256)]);
        assert!(connect("localhost", port, Duration::from_secs(5), &p).is_ok());
    }

    /// The check that runs first, replaced by one that fails with a given error.
    #[derive(Debug)]
    struct Failing(fn() -> Error);

    impl ServerCertVerifier for Failing {
        fn verify_server_cert(
            &self,
            _: &CertificateDer<'_>,
            _: &[CertificateDer<'_>],
            _: &ServerName<'_>,
            _: &[u8],
            _: UnixTime,
        ) -> Result<ServerCertVerified, Error> {
            Err((self.0)())
        }
        fn verify_tls12_signature(
            &self,
            _: &[u8],
            _: &CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            unreachable!()
        }
        fn verify_tls13_signature(
            &self,
            _: &[u8],
            _: &CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            unreachable!()
        }
        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            Vec::new()
        }
    }

    /// FR-SPOT-13: only a problem with the *certificate* can be approved. A different kind of
    /// failure is refused even for an approved fingerprint and is never offered to the operator;
    /// certificate problems of every kind are approvable and are described.
    /// trace: FR-SPOT-13
    #[test]
    fn fr_spot_13_only_certificate_problems_can_be_approved() {
        let check = |make: fn() -> Error, approved: bool| {
            let seen = Arc::new(Mutex::new(None));
            let v = PinVerifier {
                inner: Arc::new(Failing(make)),
                provider: Arc::new(rustls::crypto::ring::default_provider()),
                host: "h.example".into(),
                port: 1884,
                pins: pins(if approved {
                    &[("h.example", 1884, A_SHA256)]
                } else {
                    &[]
                }),
                seen: Arc::clone(&seen),
            };
            let cert = CertificateDer::from(A_CERT.to_vec());
            let name = ServerName::try_from("h.example").unwrap();
            let r = v.verify_server_cert(&cert, &[], &name, &[], UnixTime::now());
            let seen = seen.lock().unwrap().clone();
            (r.is_ok(), seen)
        };
        // Not certificate problems: refused, approved or not, and never offered for approval.
        let others: [fn() -> Error; 3] = [
            || Error::General("boom".into()),
            || Error::PeerMisbehaved(rustls::PeerMisbehaved::SelectedUnofferedCipherSuite),
            || Error::NoCertificatesPresented,
        ];
        for make in others {
            for approved in [false, true] {
                let (ok, seen) = check(make, approved);
                assert!(
                    !ok,
                    "a non-certificate error was accepted (approved: {approved})"
                );
                assert!(
                    seen.is_none(),
                    "a non-certificate error was offered for approval"
                );
            }
        }
        // Certificate problems: refused and offered when not approved (with a reason), accepted
        // when approved.
        type Case = (fn() -> Error, &'static str);
        let cert_errors: [Case; 5] = [
            (
                || Error::InvalidCertificate(CertificateError::UnknownIssuer),
                "unknown issuer",
            ),
            (
                || Error::InvalidCertificate(CertificateError::Expired),
                "expired",
            ),
            (
                || Error::InvalidCertificate(CertificateError::NotValidYet),
                "not valid yet",
            ),
            (
                || Error::InvalidCertificate(CertificateError::NotValidForName),
                "name does not match",
            ),
            (
                || Error::InvalidCertificate(CertificateError::BadSignature),
                "signature is bad",
            ),
        ];
        for (make, words) in cert_errors {
            let (ok, seen) = check(make, false);
            assert!(!ok);
            let info = seen.expect("offered for approval");
            assert!(info.reason.contains(words), "{} lacks {words}", info.reason);
            assert_eq!(info.sha256, A_SHA256);
            let (ok, seen) = check(make, true);
            assert!(ok, "an approved certificate with a {words} problem");
            assert!(seen.is_none());
        }
    }

    /// FR-SPOT-13: a fingerprint is not a password. A server presenting an approved certificate
    /// but signing with a different key — someone who has only the public certificate — is refused,
    /// and is not offered to the operator as something to approve.
    /// trace: FR-SPOT-13
    #[test]
    fn fr_spot_13_an_approved_certificate_still_has_to_prove_its_key() {
        // A's certificate, B's private key.
        let port = serve(A_CERT, B_KEY);
        let err = connect_to(port, &pins(&[("127.0.0.1", port, A_SHA256)]))
            .err()
            .expect("a replayed certificate must not connect");
        match err {
            ConnectError::Failed(why) => assert!(why.contains("secure connection"), "{why}"),
            other => panic!("expected a plain failure, got {other:?}"),
        }
    }

    /// FR-SPOT-13: a server that does not speak TLS, one that never answers, and a closed port are
    /// each a plain failure — bounded, never a certificate prompt.
    /// trace: FR-SPOT-13
    #[test]
    fn fr_spot_13_other_failures_are_plain_failures() {
        // Plain text where TLS was expected.
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                let _ = s.write_all(b"SSH-2.0-not-tls\r\n");
                thread::sleep(Duration::from_millis(300));
            }
        });
        match connect_to(port, &pins(&[])) {
            Err(ConnectError::Failed(why)) => assert!(why.contains("secure connection"), "{why}"),
            other => panic!("expected a failure, got {:?}", other.err()),
        }
        // Nothing listening.
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let closed = l.local_addr().unwrap().port();
        drop(l);
        match connect_to(closed, &pins(&[])) {
            Err(ConnectError::Failed(why)) => assert!(why.contains("failed"), "{why}"),
            other => panic!("expected a failure, got {:?}", other.err()),
        }
        // An address that is not a name.
        match connect("not a host name!", 1884, Duration::from_secs(1), &pins(&[])) {
            Err(ConnectError::Failed(why)) => {
                assert!(why.contains("not a valid server name"), "{why}")
            }
            other => panic!("expected a failure, got {:?}", other.err()),
        }
    }

    /// A manual probe of the real service, never run by the suite (`cargo test -p k4remote --
    /// --ignored live_psk_reporter_tls --nocapture`): one TLS handshake with PSK Reporter's port
    /// 1884, then hang up. Nothing is sent after the handshake — no MQTT, no login — and the only
    /// identifying thing in it is the server name every TLS client sends. It reports whether the
    /// certificate is trusted by the public authorities (no approval needed) or not (and then its
    /// fingerprint), which is what decides whether operators will ever meet the approval prompt.
    #[test]
    #[ignore = "contacts the real PSK Reporter server"]
    fn live_psk_reporter_tls() {
        let r = connect(
            "mqtt.pskreporter.info",
            1884,
            Duration::from_secs(10),
            &pins(&[]),
        );
        match r {
            Ok(_) => println!("PROBE: trusted by the public authorities; no approval needed"),
            Err(ConnectError::Untrusted(i)) => println!(
                "PROBE: NOT trusted ({}); SHA-256 {}",
                i.reason,
                display_fingerprint(&i.sha256)
            ),
            Err(ConnectError::Failed(why)) => println!("PROBE: failed: {why}"),
        }
    }

    /// FR-SPOT-13: the pins follow the configuration entry for entry, and an invalid entry never
    /// becomes one.
    /// trace: FR-SPOT-13
    #[test]
    fn fr_spot_13_pins_follow_the_configuration() {
        let good = k4_config::TrustedCert {
            host: "mqtt.example.org".into(),
            port: 1884,
            sha256: A_SHA256.into(),
        };
        let bad = k4_config::TrustedCert {
            sha256: "nope".into(),
            ..good.clone()
        };
        assert_eq!(pins_from(&[]), Vec::new());
        assert_eq!(
            pins_from(&[good.clone(), bad, good.clone()]),
            vec![
                Pin {
                    host: "mqtt.example.org".into(),
                    port: 1884,
                    sha256: A_SHA256.into()
                };
                2
            ]
        );
    }

    /// The fingerprint arithmetic, against values fixed elsewhere: the SHA-256 of "abc" from FIPS
    /// 180-4, and the two certificates against `openssl`.
    /// trace: FR-SPOT-13
    #[test]
    fn fr_spot_13_fingerprint_arithmetic() {
        assert_eq!(
            fingerprint(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(fingerprint(A_CERT), A_SHA256);
        assert_eq!(fingerprint(B_CERT), B_SHA256);
        assert_ne!(A_SHA256, B_SHA256);
        assert_eq!(display_fingerprint("ba7816bf8f01"), "BA:78:16:BF:8F:01");
    }
}
