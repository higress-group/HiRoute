//! Compare exact HMAC-SHA256 backends independently of model/network latency.
//! This diagnostic measures hashing only, not whole-request performance.
#![forbid(unsafe_code)]

use std::hint::black_box;
use std::time::Instant;

use hmac::{Hmac, Mac};
use ring::hmac as accelerated;
use sha2::Sha256;

const KEY: [u8; 32] = [0x37; 32];
const DOMAIN: &[u8] = b"gateway-hash-benchmark/v1";
const ROUNDS: usize = 11;
const ITERATIONS: usize = 20;

fn rustcrypto(bytes: &[u8], chunk_bytes: usize) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(&KEY).unwrap();
    mac.update(&(DOMAIN.len() as u64).to_be_bytes());
    mac.update(DOMAIN);
    for chunk in bytes.chunks(chunk_bytes) {
        mac.update(chunk);
    }
    mac.finalize().into_bytes().into()
}

fn ring(bytes: &[u8], chunk_bytes: usize) -> [u8; 32] {
    let key = accelerated::Key::new(accelerated::HMAC_SHA256, &KEY);
    let mut mac = accelerated::Context::with_key(&key);
    mac.update(&(DOMAIN.len() as u64).to_be_bytes());
    mac.update(DOMAIN);
    for chunk in bytes.chunks(chunk_bytes) {
        mac.update(chunk);
    }
    mac.sign().as_ref().try_into().unwrap()
}

fn main() {
    if cfg!(debug_assertions) {
        panic!("run this benchmark in release mode");
    }
    let payload: Vec<_> = (0..400 * 1024).map(|i| (i % 251) as u8).collect();
    // Include block/padding boundaries and fragmented updates. Both backends
    // see identical domain framing and bytes, including the empty input.
    for length in [0, 1, 55, 56, 63, 64, 65, 1024, payload.len()] {
        let expected = rustcrypto(&payload[..length], payload.len());
        for chunk in [1, 63, 64, 65, 8192, 16384] {
            assert_eq!(rustcrypto(&payload[..length], chunk), expected);
            assert_eq!(ring(&payload[..length], chunk), expected);
        }
    }
    type Hash = fn(&[u8], usize) -> [u8; 32];
    let backends: [(&str, Hash); 2] = [("rustcrypto", rustcrypto), ("ring", ring)];
    println!("backend\tbytes\tchunk_bytes\tround\tns_per_operation");
    for (bytes, chunk) in [(128, 16), (4096, 1024), (payload.len(), 16384)] {
        let input = &payload[..bytes];
        for round in 0..=ROUNDS {
            for offset in 0..backends.len() {
                let (name, hash) = backends[(round + offset) % backends.len()];
                let started = Instant::now();
                for _ in 0..ITERATIONS {
                    black_box(hash(black_box(input), chunk));
                }
                let ns = started.elapsed().as_nanos() / ITERATIONS as u128;
                if round > 0 {
                    println!("{name}\t{bytes}\t{chunk}\t{round}\t{ns}");
                }
            }
        }
    }
}
