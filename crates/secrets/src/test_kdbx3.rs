//! Test-only writer for KDBX 3.1 files, the format JetBrains IDEs use for
//! their KeePass database (`keepass` can read it but only writes KDBX 4).
//! Written from the public KDBX 3.1 format description: AES-KDF, AES-256-CBC,
//! hashed block stream, no compression, ChaCha20 for protected values.

use aes::cipher::{BlockCipherEncrypt, BlockModeEncrypt, KeyInit, KeyIvInit};
use base64::Engine as _;
use cbc::cipher::block_padding::Pkcs7;
use chacha20::cipher::StreamCipher;
use sha2::{Digest, Sha256, Sha512};

const AES_CIPHER_ID: [u8; 16] = [
    0x31, 0xc1, 0xf2, 0xe6, 0xbf, 0x71, 0x43, 0x50, 0xbe, 0x58, 0x05, 0x21, 0x6a, 0xfc, 0x5a, 0xff,
];
const ROUNDS: u64 = 100;

/// One entry: title, user name, password.
pub(crate) type Entry<'a> = (&'a str, &'a str, &'a str);

fn b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn field(header: &mut Vec<u8>, id: u8, data: &[u8]) {
    header.push(id);
    header.extend_from_slice(&(data.len() as u16).to_le_bytes());
    header.extend_from_slice(data);
}

/// A KDBX 3.1 database with `groups`: each group name holds its entries.
/// Seeds are fixed: this is a fixture, not a secure writer.
pub(crate) fn write(password: &str, groups: &[(&str, &[Entry<'_>])]) -> Vec<u8> {
    let master_seed = [1u8; 32];
    let transform_seed = [2u8; 32];
    let iv = [3u8; 16];
    let stream_key = [4u8; 32];
    let start_bytes = [5u8; 32];

    let mut out = Vec::new();
    out.extend_from_slice(&0x9AA2_D903u32.to_le_bytes());
    out.extend_from_slice(&0xB54B_FB67u32.to_le_bytes());
    out.extend_from_slice(&0x0003_0001u32.to_le_bytes());
    field(&mut out, 2, &AES_CIPHER_ID);
    field(&mut out, 3, &0u32.to_le_bytes());
    field(&mut out, 4, &master_seed);
    field(&mut out, 5, &transform_seed);
    field(&mut out, 6, &ROUNDS.to_le_bytes());
    field(&mut out, 7, &iv);
    field(&mut out, 8, &stream_key);
    field(&mut out, 9, &start_bytes);
    field(&mut out, 10, &3u32.to_le_bytes());
    field(&mut out, 0, b"\r\n\r\n");

    // Composite key, AES-KDF, master key.
    let composite = Sha256::digest(Sha256::digest(password.as_bytes()));
    let kdf = aes::Aes256::new_from_slice(&transform_seed).unwrap();
    let mut transformed = [0u8; 32];
    transformed.copy_from_slice(&composite);
    for half in transformed.chunks_mut(16) {
        let mut block = aes::Block::try_from(&*half).unwrap();
        for _ in 0..ROUNDS {
            kdf.encrypt_block(&mut block);
        }
        half.copy_from_slice(&block);
    }
    let transformed = Sha256::digest(transformed);
    let mut h = Sha256::new();
    h.update(master_seed);
    h.update(transformed);
    let master_key = h.finalize();

    // Inner stream for protected values.
    let inner = Sha512::digest(stream_key);
    let mut chacha = chacha20::ChaCha20::new_from_slices(&inner[..32], &inner[32..44]).unwrap();
    let mut protect = |plain: &str| {
        let mut data = plain.as_bytes().to_vec();
        chacha.apply_keystream(&mut data);
        b64(&data)
    };

    let time = "2026-01-01T00:00:00Z";
    let times = format!(
        "<Times><CreationTime>{time}</CreationTime><LastModificationTime>{time}</LastModificationTime>\
         <LastAccessTime>{time}</LastAccessTime><ExpiryTime>{time}</ExpiryTime><Expires>False</Expires>\
         <UsageCount>0</UsageCount><LocationChanged>{time}</LocationChanged></Times>"
    );
    let mut uuid_counter = 0u8;
    let mut uuid = || {
        uuid_counter += 1;
        b64(&[uuid_counter; 16])
    };
    let mut xml = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\" standalone=\"yes\"?><KeePassFile><Meta>\
         <Generator>fixture</Generator><DatabaseName>Fixture</DatabaseName>\
         <MemoryProtection><ProtectTitle>False</ProtectTitle><ProtectUserName>False</ProtectUserName>\
         <ProtectPassword>True</ProtectPassword><ProtectURL>False</ProtectURL><ProtectNotes>False</ProtectNotes>\
         </MemoryProtection></Meta><Root><Group><UUID>{}</UUID><Name>Root</Name>{times}",
        uuid()
    );
    for (name, entries) in groups {
        xml.push_str(&format!(
            "<Group><UUID>{}</UUID><Name>{}</Name>{times}",
            uuid(),
            xml_escape(name)
        ));
        for (title, user, pass) in *entries {
            xml.push_str(&format!(
                "<Entry><UUID>{}</UUID><IconID>0</IconID>{times}\
                 <String><Key>Title</Key><Value>{}</Value></String>\
                 <String><Key>UserName</Key><Value>{}</Value></String>\
                 <String><Key>Password</Key><Value Protected=\"True\">{}</Value></String></Entry>",
                uuid(),
                xml_escape(title),
                xml_escape(user),
                protect(pass)
            ));
        }
        xml.push_str("</Group>");
    }
    xml.push_str("</Group></Root></KeePassFile>");

    // Hashed block stream: one data block, then the empty final block.
    let data = xml.as_bytes();
    let mut payload = start_bytes.to_vec();
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&Sha256::digest(data));
    payload.extend_from_slice(&(data.len() as u32).to_le_bytes());
    payload.extend_from_slice(data);
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&[0u8; 32]);
    payload.extend_from_slice(&0u32.to_le_bytes());

    let encrypted = cbc::Encryptor::<aes::Aes256>::new_from_slices(&master_key, &iv)
        .unwrap()
        .encrypt_padded_vec::<Pkcs7>(&payload);
    out.extend_from_slice(&encrypted);
    out
}
