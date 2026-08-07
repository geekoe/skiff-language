//! `skiff-router` binary (Router Rust Migration PR 0b).
//!
//! Supports direct process identity/lifecycle smoke (unchanged from PR 0a):
//!
//! - `skiff-router --identity` prints `skiff-router <sha256-of-self>` and
//!   exits 0.
//! - `skiff-router` (no config path) prints a no-listener marker on stderr
//!   and exits 0 (frozen router-rust-process-smoke behavior).
//! - `skiff-router <config>` parses the frozen Router config, starts the
//!   public/runtime/control listeners (C-net mechanism), and shuts them down
//!   gracefully on SIGINT/SIGTERM.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::process::ExitCode;

use skiff_router::{load_router_config, run_router};
use skiff_runtime_transport::pid_lock::{PidFileGuard, PidLockError};

const SHA256_ROUND_CONSTANTS: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

const INITIAL_HASH: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|argument| argument == "--identity") {
        println!("skiff-router {}", self_sha256());
        return ExitCode::SUCCESS;
    }
    let Some(config_path) = args.first() else {
        eprintln!("skiff-router: no config path provided; no listener bound");
        return ExitCode::SUCCESS;
    };
    if args.len() != 1 || config_path.starts_with('-') {
        eprintln!("skiff-router: usage: skiff-router <router.yml> | skiff-router --identity");
        eprintln!("skiff-router: no listener bound");
        return ExitCode::FAILURE;
    }
    let config = match load_router_config(config_path) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("skiff-router: {error}");
            return ExitCode::FAILURE;
        }
    };
    let _pid_file_guard = match &config.run_dir {
        Some(run_dir) => match PidFileGuard::acquire(Path::new(run_dir), "router") {
            Ok(guard) => Some(guard),
            Err(PidLockError::AlreadyRunning { pid }) => {
                eprintln!(
                    "skiff-router: run dir {run_dir} is already in use by pid {pid}; refusing to start"
                );
                return ExitCode::FAILURE;
            }
            Err(error) => {
                eprintln!("skiff-router: failed to acquire pid file in run dir {run_dir}: {error}");
                return ExitCode::FAILURE;
            }
        },
        None => None,
    };
    match run_router(config).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("skiff-router: {error}");
            ExitCode::FAILURE
        }
    }
}

fn self_sha256() -> String {
    let executable = std::env::current_exe().expect("resolve current executable path");
    sha256_hex(&read_binary(&executable).expect("read current executable"))
}

fn read_binary(path: &Path) -> io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

pub fn sha256_hex(data: &[u8]) -> String {
    sha256(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hash = INITIAL_HASH;
    let mut message = data.to_vec();
    let bit_length = u64::try_from(data.len())
        .expect("input length must fit in u64")
        .checked_mul(8)
        .expect("input bit length must not overflow");
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_length.to_be_bytes());

    for chunk in message.chunks_exact(64) {
        let mut words = [0u32; 64];
        for (index, word) in chunk.chunks_exact(4).enumerate() {
            words[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..64 {
            let sigma0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let sigma1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(sigma0)
                .wrapping_add(words[index - 7])
                .wrapping_add(sigma1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = hash;
        for (index, constant) in SHA256_ROUND_CONSTANTS.iter().enumerate() {
            let sigma1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ ((!e) & g);
            let temporary1 = h
                .wrapping_add(sigma1)
                .wrapping_add(choose)
                .wrapping_add(*constant)
                .wrapping_add(words[index]);
            let sigma0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temporary2 = sigma0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temporary1);
            d = c;
            c = b;
            b = a;
            a = temporary1.wrapping_add(temporary2);
        }

        hash[0] = hash[0].wrapping_add(a);
        hash[1] = hash[1].wrapping_add(b);
        hash[2] = hash[2].wrapping_add(c);
        hash[3] = hash[3].wrapping_add(d);
        hash[4] = hash[4].wrapping_add(e);
        hash[5] = hash[5].wrapping_add(f);
        hash[6] = hash[6].wrapping_add(g);
        hash[7] = hash[7].wrapping_add(h);
    }

    let mut digest = [0u8; 32];
    for (index, word) in hash.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

#[cfg(test)]
mod tests {
    use super::sha256_hex;

    #[test]
    fn sha256_known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"skiff-router"),
            "f073de5e0fa5ada872e923cd80c0954e46cf96946895aaa56ce4f0aa6b70473e"
        );
    }

    #[test]
    fn sha256_digest_is_lowercase_hex() {
        let digest = sha256_hex(b"identity");
        assert_eq!(digest.len(), 64);
        assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(digest.to_lowercase(), digest);
    }
}
