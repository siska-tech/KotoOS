//! SPKI-pinned TLS 1.3 verifier for KOTO-0245 (probe and, since the
//! `app_fetch_https` feature, the product HTTPS session).
//!
//! Trust is rooted exclusively in the signed package's current/successor SPKI
//! pins, not SNTP or certificate validity time. Pin acceptance is followed by
//! mandatory P-256 CertificateVerify validation over the TLS 1.3 transcript.

use embedded_tls::{
    Aes128GcmSha256, CertificateEntryRef, CertificateRef, CertificateVerifyRef, Sha256,
    SignatureScheme, TlsError, TlsVerifier,
};
use koto_core::{
    extract_certificate_spki_der, extract_p256_public_key_from_spki_der,
    verify_p256_tls13_certificate_signature, FetchPinSet,
};
use sha2::Digest;

/// Records a TLS handshake milestone (KOTO-0245 diagnostics) in the product
/// path; a no-op in the isolated probe build.
#[cfg(all(
    feature = "app_fetch_https",
    any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w")
))]
fn phase(milestone: u32) {
    crate::firmware::wifi_residency::record_fetch_tls_phase(milestone);
}
#[cfg(not(all(
    feature = "app_fetch_https",
    any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w")
)))]
fn phase(_milestone: u32) {}

pub struct PinnedP256Verifier {
    pins: FetchPinSet,
    public_key: [u8; 65],
    transcript_hash: Option<[u8; 32]>,
    certificate_accepted: bool,
}

impl PinnedP256Verifier {
    pub const fn new(pins: FetchPinSet) -> Self {
        Self {
            pins,
            public_key: [0; 65],
            transcript_hash: None,
            certificate_accepted: false,
        }
    }

    fn reset_peer(&mut self) {
        self.public_key.fill(0);
        self.transcript_hash = None;
        self.certificate_accepted = false;
    }
}

impl TlsVerifier<Aes128GcmSha256> for PinnedP256Verifier {
    fn set_hostname_verification(&mut self, hostname: &str) -> Result<(), TlsError> {
        phase(10);
        if hostname.is_empty() || hostname.len() > 253 || !hostname.is_ascii() {
            return Err(TlsError::InvalidCertificate);
        }
        Ok(())
    }

    fn verify_certificate(
        &mut self,
        transcript: &Sha256,
        cert: CertificateRef,
    ) -> Result<(), TlsError> {
        phase(11);
        self.reset_peer();
        let certificate = match cert.entries.first() {
            Some(CertificateEntryRef::X509(certificate)) => *certificate,
            _ => return Err(TlsError::InvalidCertificate),
        };
        let spki =
            extract_certificate_spki_der(certificate).map_err(|_| TlsError::InvalidCertificate)?;
        phase(12);
        let digest: [u8; 32] = Sha256::digest(spki).into();
        if !self.pins.matches_digest(&digest) {
            return Err(TlsError::InvalidCertificate);
        }
        phase(13);
        let public_key = extract_p256_public_key_from_spki_der(spki)
            .map_err(|_| TlsError::InvalidCertificate)?;
        self.public_key.copy_from_slice(public_key);
        // CertificateVerify covers the transcript through Certificate. Store
        // only its digest in the async verifier state so `verify_signature`
        // does not retain a full SHA-256 state across the P-256 stack peak.
        self.transcript_hash = Some(transcript.clone().finalize().into());
        self.certificate_accepted = true;
        phase(14);
        Ok(())
    }

    #[inline(never)]
    fn verify_signature(&mut self, verify: CertificateVerifyRef) -> Result<(), TlsError> {
        phase(15);
        if verify.signature_scheme != SignatureScheme::EcdsaSecp256r1Sha256
            || !self.certificate_accepted
        {
            self.reset_peer();
            return Err(TlsError::InvalidSignatureScheme);
        }
        let Some(transcript_hash) = self.transcript_hash.take() else {
            self.reset_peer();
            return Err(TlsError::InvalidSignature);
        };
        self.certificate_accepted = false;

        let result = verify_p256_tls13_certificate_signature(
            &self.public_key,
            &transcript_hash,
            verify.signature,
        )
        .then_some(())
        .ok_or(TlsError::InvalidSignature);
        self.public_key.fill(0);
        if result.is_ok() {
            phase(16);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use embedded_tls::CertificateEntryRef;
    use koto_core::SpkiSha256;
    use p256::ecdsa::{signature::Signer, SigningKey};

    fn fixture() -> ([u8; 117], FetchPinSet, SigningKey) {
        let signing_key = SigningKey::from_bytes((&[7u8; 32]).into()).unwrap();
        let encoded_key = signing_key.verifying_key().to_encoded_point(false);
        let mut spki = [0u8; 91];
        spki[..26].copy_from_slice(&[
            0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06,
            0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
        ]);
        spki[26..].copy_from_slice(encoded_key.as_bytes());

        let mut certificate = [0u8; 117];
        certificate[..20].copy_from_slice(&[
            0x30, 0x73, 0x30, 0x6b, 0xa0, 0x03, 0x02, 0x01, 0x02, 0x02, 0x01, 0x01, 0x30, 0x00,
            0x30, 0x00, 0x30, 0x00, 0x30, 0x00,
        ]);
        certificate[20..111].copy_from_slice(&spki);
        certificate[111..].copy_from_slice(&[0x30, 0x00, 0x03, 0x02, 0x00, 0xaa]);

        let mut pins = FetchPinSet::empty();
        pins.push(SpkiSha256::from_bytes(Sha256::digest(spki).into()))
            .unwrap();
        (certificate, pins, signing_key)
    }

    #[test]
    fn pin_and_certificate_verify_are_both_required() {
        let (certificate, pins, signing_key) = fixture();
        let mut transcript = Sha256::new();
        transcript.update(b"bounded transcript");
        let mut verifier = PinnedP256Verifier::new(pins);
        let mut cert = CertificateRef::with_context(&[]);
        cert.add(CertificateEntryRef::X509(&certificate)).unwrap();
        verifier.verify_certificate(&transcript, cert).unwrap();

        let mut message = [0u8; 130];
        message[..64].fill(0x20);
        message[64..98].copy_from_slice(b"TLS 1.3, server CertificateVerify\x00");
        message[98..].copy_from_slice(&transcript.finalize());
        let signature: Signature = signing_key.sign(&message);
        let signature_der = signature.to_der();
        verifier
            .verify_signature(CertificateVerifyRef {
                signature_scheme: SignatureScheme::EcdsaSecp256r1Sha256,
                signature: signature_der.as_bytes(),
            })
            .unwrap();

        let mut wrong_pin = FetchPinSet::empty();
        wrong_pin.push(SpkiSha256::from_bytes([0xff; 32])).unwrap();
        let mut rejected = PinnedP256Verifier::new(wrong_pin);
        let mut cert = CertificateRef::with_context(&[]);
        cert.add(CertificateEntryRef::X509(&certificate)).unwrap();
        let mut transcript = Sha256::new();
        transcript.update(b"bounded transcript");
        assert_eq!(
            rejected.verify_certificate(&transcript, cert),
            Err(TlsError::InvalidCertificate)
        );
    }
}
