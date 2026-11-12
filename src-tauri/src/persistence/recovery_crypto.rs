//! Independent encrypted-snapshot format foundation; no filesystem publication.
//!
//! Callers must supply pinned finite handles, validate SQLite separately, serialize
//! KDF operations, check cancellation through their I/O wrappers, discard partial
//! private staging on error, and finish sync/atomic publication themselves.
//! age 0.12.1's passphrase constructors calibrate before their setters run. The
//! actual file KDF is fixed/capped below; constructor work is not strictly fixed.
use crate::domain::{AppError, AppResult};
use age::{Decryptor, Encryptor};
use base64::{Engine, engine::general_purpose::STANDARD_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use zeroize::Zeroizing;

pub(crate) const MAX_PAYLOAD_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_HEADER_BYTES: usize = 8 * 1024;
const MAX_MANIFEST_BYTES: usize = 16 * 1024;
const BUFFER_BYTES: usize = 64 * 1024;
const WORK_FACTOR: u8 = 18;
const MAGIC: &[u8] = b"JOINERY-METADATA\0";
const MAX_PLAINTEXT_BYTES: u64 =
    MAX_PAYLOAD_BYTES + MAX_MANIFEST_BYTES as u64 + MAGIC.len() as u64 + 4;
// age's 64 KiB plaintext chunks each add a 16-byte authentication tag.
pub(crate) const MAX_CIPHERTEXT_BYTES: u64 = MAX_PLAINTEXT_BYTES
    + MAX_PLAINTEXT_BYTES.div_ceil(BUFFER_BYTES as u64) * 16
    + MAX_HEADER_BYTES as u64
    + 16;

/// No Debug/Serialize/Clone: a passphrase is confined to this native operation.
pub(crate) struct BackupPassphrase(Zeroizing<String>);

impl BackupPassphrase {
    pub(crate) fn new(value: Zeroizing<String>) -> AppResult<Self> {
        if value.len() > 4096 || !(12..=1024).contains(&value.chars().count()) {
            return Err(AppError::invalid(
                "A backup passphrase must contain 12–1024 Unicode characters",
            ));
        }
        Ok(Self(value))
    }

    fn secret(&self) -> age::secrecy::SecretString {
        // age's SecretString also zeroizes its bounded owned copy on drop.
        age::secrecy::SecretString::from(self.0.as_str().to_owned())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SnapshotManifest {
    pub(crate) format_version: u32,
    pub(crate) schema_version: u32,
    pub(crate) payload_bytes: u64,
    pub(crate) payload_sha256: String,
}

impl SnapshotManifest {
    fn validate(&self) -> AppResult<()> {
        if self.format_version != 1
            || !(1..=4).contains(&self.schema_version)
            || self.payload_bytes > MAX_PAYLOAD_BYTES
            || self.payload_sha256.len() != 64
            || !self
                .payload_sha256
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(invalid_file());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EncryptedSnapshotInfo {
    pub(crate) manifest: SnapshotManifest,
    pub(crate) ciphertext_bytes: u64,
    pub(crate) ciphertext_sha256: String,
}

pub(crate) type DecryptedSnapshotInfo = EncryptedSnapshotInfo;

fn invalid_file() -> AppError {
    AppError::invalid("The encrypted metadata backup is invalid or unsupported")
}

fn decrypt_error() -> AppError {
    AppError::invalid("Could not decrypt the metadata backup; check its passphrase and file")
}

fn io_error(_: io::Error) -> AppError {
    AppError::internal("Could not stream the metadata backup")
}

fn size_error() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "metadata backup size limit")
}

/// Tracks actual successful bytes without buffering the encrypted file.
struct CipherIo<T> {
    inner: T,
    bytes: u64,
    hash: Sha256,
}

impl<T> CipherIo<T> {
    fn new(inner: T) -> Self {
        Self {
            inner,
            bytes: 0,
            hash: Sha256::new(),
        }
    }

    fn info(&self, manifest: SnapshotManifest) -> EncryptedSnapshotInfo {
        EncryptedSnapshotInfo {
            manifest,
            ciphertext_bytes: self.bytes,
            ciphertext_sha256: hex::encode(self.hash.clone().finalize()),
        }
    }
}

impl<R: Read> Read for CipherIo<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.bytes == MAX_CIPHERTEXT_BYTES {
            // Never manufacture EOF at the cap: extra physical bytes are an error.
            let mut extra = [0];
            return if self.inner.read(&mut extra)? == 0 {
                Ok(0)
            } else {
                Err(size_error())
            };
        }
        let remaining = usize::try_from(MAX_CIPHERTEXT_BYTES - self.bytes).unwrap_or(usize::MAX);
        let limit = buffer.len().min(remaining);
        let count = self.inner.read(&mut buffer[..limit])?;
        self.hash.update(&buffer[..count]);
        self.bytes += count as u64;
        Ok(count)
    }
}

impl<W: Write> Write for CipherIo<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() as u64 > MAX_CIPHERTEXT_BYTES - self.bytes {
            return Err(size_error());
        }
        let count = self.inner.write(bytes)?;
        self.hash.update(&bytes[..count]);
        self.bytes += count as u64;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn payload_digest<R: Read>(input: &mut R) -> AppResult<(u64, String)> {
    let mut buffer = Zeroizing::new([0u8; BUFFER_BYTES]);
    let mut bytes = 0u64;
    let mut hash = Sha256::new();
    loop {
        let count = input.read(&mut buffer[..]).map_err(io_error)?;
        if count == 0 {
            break;
        }
        bytes = bytes
            .checked_add(count as u64)
            .filter(|n| *n <= MAX_PAYLOAD_BYTES)
            .ok_or_else(invalid_file)?;
        hash.update(&buffer[..count]);
    }
    Ok((bytes, hex::encode(hash.finalize())))
}

pub(crate) fn encrypt_snapshot<R: Read + Seek, W: Write>(
    snapshot: &mut R,
    schema_version: u32,
    passphrase: &BackupPassphrase,
    output: &mut W,
) -> AppResult<EncryptedSnapshotInfo> {
    encrypt_with_work_factor(snapshot, schema_version, passphrase, output, WORK_FACTOR)
}

/// Cheap repeatable SQLite workflow fixtures, never a production runtime knob.
#[cfg(test)]
pub(super) fn encrypt_snapshot_fixture<R: Read + Seek, W: Write>(
    snapshot: &mut R,
    schema_version: u32,
    passphrase: &BackupPassphrase,
    output: &mut W,
) -> AppResult<EncryptedSnapshotInfo> {
    encrypt_with_work_factor(snapshot, schema_version, passphrase, output, 2)
}

fn encrypt_with_work_factor<R: Read + Seek, W: Write>(
    snapshot: &mut R,
    schema_version: u32,
    passphrase: &BackupPassphrase,
    output: &mut W,
    work_factor: u8,
) -> AppResult<EncryptedSnapshotInfo> {
    if !(1..=4).contains(&schema_version) || !(1..=WORK_FACTOR).contains(&work_factor) {
        return Err(invalid_file());
    }
    snapshot.seek(SeekFrom::Start(0)).map_err(io_error)?;
    let (payload_bytes, payload_sha256) = payload_digest(snapshot)?;
    let manifest = SnapshotManifest {
        format_version: 1,
        schema_version,
        payload_bytes,
        payload_sha256,
    };
    manifest.validate()?;
    let encoded = Zeroizing::new(serde_json::to_vec(&manifest).map_err(|_| invalid_file())?);
    if encoded.len() > MAX_MANIFEST_BYTES {
        return Err(invalid_file());
    }
    snapshot.seek(SeekFrom::Start(0)).map_err(io_error)?;
    let mut recipient = age::scrypt::Recipient::new(passphrase.secret());
    recipient.set_work_factor(work_factor);
    let encryptor = Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
        .map_err(|_| invalid_file())?;
    let mut cipher = CipherIo::new(output);
    let mut writer = encryptor.wrap_output(&mut cipher).map_err(io_error)?;
    writer.write_all(MAGIC).map_err(io_error)?;
    writer
        .write_all(&(encoded.len() as u32).to_be_bytes())
        .map_err(io_error)?;
    writer.write_all(&encoded).map_err(io_error)?;
    let mut buffer = Zeroizing::new([0u8; BUFFER_BYTES]);
    let mut hash = Sha256::new();
    let mut copied = 0u64;
    loop {
        let count = snapshot.read(&mut buffer[..]).map_err(io_error)?;
        if count == 0 {
            break;
        }
        copied = copied
            .checked_add(count as u64)
            .filter(|n| *n <= manifest.payload_bytes)
            .ok_or_else(invalid_file)?;
        hash.update(&buffer[..count]);
        writer.write_all(&buffer[..count]).map_err(io_error)?;
    }
    if copied != manifest.payload_bytes || hex::encode(hash.finalize()) != manifest.payload_sha256 {
        return Err(invalid_file());
    }
    // Only finalized output can be handed back for sync/atomic publication.
    writer.finish().map_err(io_error)?;
    Ok(cipher.info(manifest))
}

fn bounded_header<R: Read>(input: &mut R) -> AppResult<Zeroizing<Vec<u8>>> {
    let mut header = Zeroizing::new(Vec::with_capacity(256));
    let mut line_start = 0;
    loop {
        if header.len() >= MAX_HEADER_BYTES {
            return Err(invalid_file());
        }
        let mut byte = [0];
        input.read_exact(&mut byte).map_err(|_| invalid_file())?;
        header.push(byte[0]);
        if byte[0] != b'\n' {
            continue;
        }
        let mac = header[line_start..].starts_with(b"--- ");
        line_start = header.len();
        if mac {
            break;
        }
    }
    let text = std::str::from_utf8(&header).map_err(|_| invalid_file())?;
    let lines: Vec<_> = text.split_terminator('\n').collect();
    if lines.len() != 4 || lines[0] != "age-encryption.org/v1" {
        return Err(invalid_file());
    }
    let args: Vec<_> = lines[1].split(' ').collect();
    if args.len() != 4 || args[0] != "->" || args[1] != "scrypt" {
        return Err(invalid_file());
    }
    let factor = args[3].parse::<u8>().map_err(|_| invalid_file())?;
    if !(1..=WORK_FACTOR).contains(&factor) || factor.to_string() != args[3] {
        return Err(invalid_file());
    }
    for (encoded, expected) in [
        (args[2], 16),
        (lines[2], 32),
        (lines[3].strip_prefix("--- ").ok_or_else(invalid_file)?, 32),
    ] {
        let bytes = STANDARD_NO_PAD
            .decode(encoded)
            .map_err(|_| invalid_file())?;
        if bytes.len() != expected || STANDARD_NO_PAD.encode(&bytes) != encoded {
            return Err(invalid_file());
        }
    }
    // Include the fixed-size nonce in the pre-scanned bounded prefix. No age
    // parser or KDF runs until the complete supported header has been checked.
    let mut nonce = [0; 16];
    input.read_exact(&mut nonce).map_err(|_| invalid_file())?;
    header.extend_from_slice(&nonce);
    Ok(header)
}

pub(crate) fn decrypt_snapshot<R: Read, W: Write>(
    input: &mut R,
    passphrase: &BackupPassphrase,
    staging: &mut W,
) -> AppResult<DecryptedSnapshotInfo> {
    let mut cipher = CipherIo::new(input);
    let header = bounded_header(&mut cipher)?;
    let prefix = Cursor::new(&header[..]);
    let decryptor = Decryptor::new(prefix.chain(&mut cipher)).map_err(|_| decrypt_error())?;
    if !decryptor.is_scrypt() {
        return Err(invalid_file());
    }
    let mut identity = age::scrypt::Identity::new(passphrase.secret());
    identity.set_max_work_factor(WORK_FACTOR);
    let mut reader = decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .map_err(|_| decrypt_error())?;
    let mut magic = [0; MAGIC.len()];
    reader.read_exact(&mut magic).map_err(|_| decrypt_error())?;
    if magic.as_slice() != MAGIC {
        return Err(invalid_file());
    }
    let mut length = [0; 4];
    reader
        .read_exact(&mut length)
        .map_err(|_| decrypt_error())?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > MAX_MANIFEST_BYTES {
        return Err(invalid_file());
    }
    let mut encoded = Zeroizing::new(vec![0; length]);
    reader
        .read_exact(&mut encoded)
        .map_err(|_| decrypt_error())?;
    let manifest: SnapshotManifest =
        serde_json::from_slice(&encoded).map_err(|_| invalid_file())?;
    manifest.validate()?;
    let mut buffer = Zeroizing::new([0u8; BUFFER_BYTES]);
    let mut hash = Sha256::new();
    let mut left = manifest.payload_bytes;
    while left != 0 {
        let limit = left.min(BUFFER_BYTES as u64) as usize;
        let count = reader
            .read(&mut buffer[..limit])
            .map_err(|_| decrypt_error())?;
        if count == 0 {
            return Err(decrypt_error());
        }
        hash.update(&buffer[..count]);
        staging.write_all(&buffer[..count]).map_err(io_error)?;
        left -= count as u64;
    }
    // Reading *past* the advertised payload authenticates the final chunk even
    // when it is exactly full; extra plaintext and trailing ciphertext fail.
    let mut extra = [0];
    if reader.read(&mut extra).map_err(|_| decrypt_error())? != 0 {
        return Err(invalid_file());
    }
    drop(reader);
    if cipher.read(&mut extra).map_err(|_| decrypt_error())? != 0 {
        return Err(invalid_file());
    }
    if hex::encode(hash.finalize()) != manifest.payload_sha256 {
        return Err(invalid_file());
    }
    Ok(cipher.info(manifest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passphrase() -> BackupPassphrase {
        BackupPassphrase::new(Zeroizing::new("separate backup passphrase".into())).unwrap()
    }

    fn encrypted(payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        encrypt_snapshot_fixture(&mut Cursor::new(payload), 4, &passphrase(), &mut bytes).unwrap();
        bytes
    }

    fn encrypted_plaintext(plaintext: &[u8]) -> Vec<u8> {
        let mut recipient = age::scrypt::Recipient::new(passphrase().secret());
        recipient.set_work_factor(2);
        let encryptor =
            Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient)).unwrap();
        let mut bytes = Vec::new();
        let mut writer = encryptor.wrap_output(&mut bytes).unwrap();
        writer.write_all(plaintext).unwrap();
        writer.finish().unwrap();
        bytes
    }

    fn framed(manifest: &[u8], payload: &[u8]) -> Vec<u8> {
        let mut plaintext = MAGIC.to_vec();
        plaintext.extend_from_slice(&(manifest.len() as u32).to_be_bytes());
        plaintext.extend_from_slice(manifest);
        plaintext.extend_from_slice(payload);
        encrypted_plaintext(&plaintext)
    }

    #[test]
    fn recovery_crypto_passphrase_bounds_count_unicode_scalars() {
        for value in ["x".repeat(11), "😀".repeat(1025)] {
            assert!(BackupPassphrase::new(Zeroizing::new(value)).is_err());
        }
        for value in ["x".repeat(12), "😀".repeat(1024)] {
            assert!(BackupPassphrase::new(Zeroizing::new(value)).is_ok());
        }
    }

    #[test]
    fn recovery_crypto_stream_round_trip_crosses_chunks_and_matches_cipher_digest() {
        let payload = vec![0xA5; BUFFER_BYTES * 3 + 71];
        let mut bytes = Vec::new();
        let original =
            encrypt_with_work_factor(&mut Cursor::new(&payload), 4, &passphrase(), &mut bytes, 2)
                .unwrap();
        assert_eq!(original.ciphertext_bytes, bytes.len() as u64);
        assert_eq!(
            original.ciphertext_sha256,
            hex::encode(Sha256::digest(&bytes))
        );
        let mut restored = Vec::new();
        assert_eq!(
            decrypt_snapshot(&mut Cursor::new(&bytes), &passphrase(), &mut restored).unwrap(),
            original
        );
        assert_eq!(restored, payload);
    }

    #[test]
    fn recovery_crypto_rejects_wrong_passphrase_tamper_truncation_and_trailing_bytes() {
        let bytes = encrypted(&vec![0x5A; BUFFER_BYTES + 31]);
        let wrong =
            BackupPassphrase::new(Zeroizing::new("another backup passphrase".into())).unwrap();
        assert!(decrypt_snapshot(&mut Cursor::new(&bytes), &wrong, &mut io::sink()).is_err());
        for end in [0, 20, bytes.len() - 1, bytes.len() - 16] {
            assert!(
                decrypt_snapshot(
                    &mut Cursor::new(&bytes[..end]),
                    &passphrase(),
                    &mut io::sink()
                )
                .is_err()
            );
        }
        for index in [80, bytes.len() / 2, bytes.len() - 1] {
            let mut damaged = bytes.clone();
            damaged[index] ^= 1;
            assert!(
                decrypt_snapshot(&mut Cursor::new(damaged), &passphrase(), &mut io::sink())
                    .is_err()
            );
        }
        let mut trailing = bytes;
        trailing.extend_from_slice(b"trailing content");
        assert!(
            decrypt_snapshot(&mut Cursor::new(trailing), &passphrase(), &mut io::sink()).is_err()
        );
    }

    #[test]
    fn recovery_crypto_rejects_unsupported_or_excessive_header_before_output() {
        for bytes in [
            b"age-encryption.org/v1\n-> X25519 fake\nbody\n--- mac\n".to_vec(),
            vec![b'X'; MAX_HEADER_BYTES + 1],
            b"age-encryption.org/v1\n-> scrypt AAAAAAAAAAAAAAAAAAAAAA 19\nAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n--- AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n".to_vec(),
        ] {
            let mut output = Vec::new();
            assert!(decrypt_snapshot(&mut Cursor::new(bytes), &passphrase(), &mut output).is_err());
            assert!(output.is_empty());
        }
    }

    #[test]
    fn recovery_crypto_closed_manifest_rejects_unknown_versions_fields_and_sizes() {
        let base = SnapshotManifest {
            format_version: 1,
            schema_version: 4,
            payload_bytes: 0,
            payload_sha256: hex::encode(Sha256::digest([])),
        };
        for version in [0, 2, u32::MAX] {
            let mut value = base.clone();
            value.format_version = version;
            assert!(value.validate().is_err());
        }
        for version in [0, 5, u32::MAX] {
            let mut value = base.clone();
            value.schema_version = version;
            assert!(value.validate().is_err());
        }
        let mut value = base.clone();
        value.payload_bytes = MAX_PAYLOAD_BYTES + 1;
        assert!(value.validate().is_err());
        let mut value = base.clone();
        value.payload_sha256 = "A".repeat(64);
        assert!(value.validate().is_err());
        let mut value = serde_json::to_value(base).unwrap();
        value["extra"] = true.into();
        assert!(serde_json::from_value::<SnapshotManifest>(value).is_err());
    }

    #[test]
    fn recovery_crypto_authenticated_manifest_and_payload_must_be_exact() {
        let payload = b"fixture payload";
        let base = SnapshotManifest {
            format_version: 1,
            schema_version: 4,
            payload_bytes: payload.len() as u64,
            payload_sha256: hex::encode(Sha256::digest(payload)),
        };
        let mut extra_field = serde_json::to_value(&base).unwrap();
        extra_field["unexpected"] = true.into();
        let mut too_large = base.clone();
        too_large.payload_bytes = MAX_PAYLOAD_BYTES + 1;
        let mut digest_mismatch = base.clone();
        digest_mismatch.payload_sha256 = "0".repeat(64);
        for manifest in [
            serde_json::to_vec(&extra_field).unwrap(),
            serde_json::to_vec(&too_large).unwrap(),
            serde_json::to_vec(&digest_mismatch).unwrap(),
            vec![b' '; MAX_MANIFEST_BYTES + 1],
        ] {
            assert!(
                decrypt_snapshot(
                    &mut Cursor::new(framed(&manifest, payload)),
                    &passphrase(),
                    &mut io::sink()
                )
                .is_err()
            );
        }
        let encoded = serde_json::to_vec(&base).unwrap();
        let mut extra_plaintext = payload.to_vec();
        extra_plaintext.push(0);
        for content in [payload[..payload.len() - 1].to_vec(), extra_plaintext] {
            assert!(
                decrypt_snapshot(
                    &mut Cursor::new(framed(&encoded, &content)),
                    &passphrase(),
                    &mut io::sink()
                )
                .is_err()
            );
        }
    }

    #[test]
    fn recovery_crypto_authenticates_exact_full_final_chunk_and_rejects_reordering() {
        let mut manifest = SnapshotManifest {
            format_version: 1,
            schema_version: 4,
            payload_bytes: 1,
            payload_sha256: "0".repeat(64),
        };
        // Find an exact full final plaintext chunk, including manifest framing.
        for _ in 0..4 {
            let encoded = serde_json::to_vec(&manifest).unwrap();
            manifest.payload_bytes = (BUFFER_BYTES - MAGIC.len() - 4 - encoded.len()) as u64;
        }
        let payload = vec![0x6B; manifest.payload_bytes as usize];
        manifest.payload_sha256 = hex::encode(Sha256::digest(&payload));
        let encoded = serde_json::to_vec(&manifest).unwrap();
        assert_eq!(
            MAGIC.len() + 4 + encoded.len() + payload.len(),
            BUFFER_BYTES
        );
        let bytes = framed(&encoded, &payload);
        assert!(decrypt_snapshot(&mut Cursor::new(&bytes), &passphrase(), &mut io::sink()).is_ok());
        assert!(
            decrypt_snapshot(
                &mut Cursor::new(&bytes[..bytes.len() - 1]),
                &passphrase(),
                &mut io::sink()
            )
            .is_err()
        );
        let mut trailing = bytes;
        trailing.push(0);
        assert!(
            decrypt_snapshot(&mut Cursor::new(trailing), &passphrase(), &mut io::sink()).is_err()
        );
        let bytes = encrypted(&vec![0x42; BUFFER_BYTES * 3]);
        let mut cursor = Cursor::new(&bytes);
        let header = bounded_header(&mut cursor).unwrap();
        let start = header.len();
        let chunk = BUFFER_BYTES + 16;
        let mut reordered = bytes.clone();
        reordered[start..start + chunk].copy_from_slice(&bytes[start + chunk..start + chunk * 2]);
        reordered[start + chunk..start + chunk * 2].copy_from_slice(&bytes[start..start + chunk]);
        assert!(
            decrypt_snapshot(&mut Cursor::new(reordered), &passphrase(), &mut io::sink()).is_err()
        );
    }

    #[test]
    fn recovery_crypto_physical_cipher_cap_never_synthesizes_eof() {
        let mut excess = CipherIo::new(Cursor::new([1]));
        excess.bytes = MAX_CIPHERTEXT_BYTES;
        assert!(excess.read(&mut [0]).is_err());
        let mut exact = CipherIo::new(Cursor::new([]));
        exact.bytes = MAX_CIPHERTEXT_BYTES;
        assert_eq!(exact.read(&mut [0]).unwrap(), 0);
        let mut output = Vec::new();
        let mut writer = CipherIo::new(&mut output);
        writer.bytes = MAX_CIPHERTEXT_BYTES - 1;
        assert!(writer.write(&[1, 2]).is_err());
        assert!(output.is_empty());
    }

    #[test]
    fn recovery_crypto_io_and_changed_snapshot_never_return_complete_evidence() {
        struct FailingWriter;
        impl Write for FailingWriter {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("fixture write failure"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        assert!(
            encrypt_snapshot_fixture(
                &mut Cursor::new(b"payload"),
                4,
                &passphrase(),
                &mut FailingWriter
            )
            .is_err()
        );
        assert!(
            decrypt_snapshot(
                &mut Cursor::new(encrypted(b"payload")),
                &passphrase(),
                &mut FailingWriter
            )
            .is_err()
        );
        struct ChangingSnapshot {
            input: Cursor<Vec<u8>>,
            seeks: usize,
        }
        impl Read for ChangingSnapshot {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                let count = self.input.read(buffer)?;
                if self.seeks > 1 && count != 0 {
                    buffer[0] ^= 1;
                }
                Ok(count)
            }
        }
        impl Seek for ChangingSnapshot {
            fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
                self.seeks += 1;
                self.input.seek(position)
            }
        }
        let mut source = ChangingSnapshot {
            input: Cursor::new(vec![1; BUFFER_BYTES * 2]),
            seeks: 0,
        };
        let mut partial = Vec::new();
        assert!(encrypt_snapshot_fixture(&mut source, 4, &passphrase(), &mut partial).is_err());
        assert!(
            decrypt_snapshot(&mut Cursor::new(partial), &passphrase(), &mut io::sink()).is_err()
        );
    }

    #[test]
    #[ignore = "Real log-N18 KDF resource/latency acceptance; lead runs serially"]
    fn recovery_crypto_default_work_factor_real_round_trip() {
        let mut encrypted = Vec::new();
        let original = encrypt_snapshot(
            &mut Cursor::new(b"SQLite snapshot fixture"),
            4,
            &passphrase(),
            &mut encrypted,
        )
        .unwrap();
        let mut output = Vec::new();
        assert_eq!(
            decrypt_snapshot(&mut Cursor::new(encrypted), &passphrase(), &mut output).unwrap(),
            original
        );
        assert_eq!(output, b"SQLite snapshot fixture");
    }
}
