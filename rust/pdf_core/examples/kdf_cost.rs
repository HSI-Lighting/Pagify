//! What the key derivation actually costs.
//!
//! The plan proposes ~48 MiB / t=3 as a starting point and says to measure it
//! on a low-end Android device before calling it settled. This measures the
//! desktop end, which is the cheap half of that answer — a phone will be
//! several times slower, and the number that matters is the one taken there.
//!
//! Run in **release**. A debug Argon2 is not a measurement of anything.
//!
//! ```text
//! cargo run --release --example kdf_cost
//! ```

use std::time::Instant;

use pdf_core::crypto::kdf::{Argon2id, KdfParams, KeyDerivation};

fn main() {
    let salt = [0x5au8; 16];
    println!("  memory   passes   lanes      time");

    for params in [
        KdfParams { memory_kib: 16 * 1024, time: 2, lanes: 1 },
        KdfParams { memory_kib: 32 * 1024, time: 3, lanes: 1 },
        // The proposed default.
        KdfParams { memory_kib: 48 * 1024, time: 3, lanes: 1 },
        KdfParams { memory_kib: 64 * 1024, time: 3, lanes: 1 },
        KdfParams { memory_kib: 96 * 1024, time: 4, lanes: 1 },
    ] {
        // Three runs, slowest reported: a KDF that is usually fast and
        // occasionally slow is experienced as slow.
        let mut worst = 0.0f64;
        for _ in 0..3 {
            let started = Instant::now();
            Argon2id.derive(b"a passcode of ordinary length", &salt, &params).expect("derive");
            worst = worst.max(started.elapsed().as_secs_f64());
        }

        println!(
            "  {:>4} MiB   {:>4}   {:>5}   {:>7.0} ms{}",
            params.memory_kib / 1024,
            params.time,
            params.lanes,
            worst * 1000.0,
            if params.memory_kib == 48 * 1024 { "   <- proposed default" } else { "" }
        );
    }

    println!(
        "\n  Desktop only. Argon2id is memory-hard by design, so a phone with slower\n  \
         memory is the number that decides this — and it is paid once per document\n  \
         open, not once per item."
    );
}
