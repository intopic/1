//! Benchmark: Montgomery CIOS portabël kundrejt AVX-512 IFMA.
//!
//! Ekzekutim: `cargo bench -p triad-core --bench montgomery_bench`
//! Mat kohën (ns) dhe ciklet e përafërta (`rdtsc` në x86_64) për
//! `mul` dhe `exp` (eksponent 2048-bit, i cili simulon një operacion privat RSA).

use std::hint::black_box;
use std::time::Instant;

use triad_core::MontgomeryEngine;

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[cfg(target_arch = "x86_64")]
fn cycles() -> u64 {
    // SAFETY: rdtsc është gjithmonë i disponueshëm në x86_64.
    unsafe { core::arch::x86_64::_rdtsc() }
}
#[cfg(not(target_arch = "x86_64"))]
fn cycles() -> u64 {
    0
}

fn bench<F: FnMut()>(name: &str, iters: u32, mut f: F) {
    for _ in 0..(iters / 10).max(1) {
        f();
    }
    let c0 = cycles();
    let t0 = Instant::now();
    for _ in 0..iters {
        f();
    }
    let ns = t0.elapsed().as_nanos() as f64 / iters as f64;
    let cyc = cycles().wrapping_sub(c0) as f64 / iters as f64;
    println!("{name:<44} {ns:>12.0} ns/op {cyc:>14.0} cikle/op");
}

fn main() {
    println!("backend i zbuluar: {}", triad_core::simd::detect());
    let mut s = 0x9E37_79B9_7F4A_7C15u64;
    for bits in [2048usize, 4096] {
        let n = bits / 64;
        let mut m: Vec<u64> = (0..n).map(|_| xorshift(&mut s)).collect();
        m[0] |= 1;
        m[n - 1] |= 1 << 63;
        let eng = MontgomeryEngine::new(&m);
        let mut a: Vec<u64> = (0..n).map(|_| xorshift(&mut s)).collect();
        a[n - 1] >>= 1;
        let e: Vec<u64> = (0..n).map(|_| xorshift(&mut s)).collect();
        let mut out = vec![0u64; n];

        bench(&format!("RSA-{bits} mul (CIOS portabël)"), 20_000, || {
            eng.mul_cios_portable(black_box(&a), black_box(&a), &mut out)
        });
        let it = if bits == 2048 { 50 } else { 8 };
        bench(
            &format!("RSA-{bits} exp (portabël, kohë konstante)"),
            it,
            || eng.exp_portable(black_box(&a), black_box(&e), &mut out),
        );
        #[cfg(target_arch = "x86_64")]
        if triad_core::simd::avx512::is_available() {
            let avx = triad_core::simd::avx512::Avx512Montgomery::new(&m);
            bench(
                &format!("RSA-{bits} exp (AVX-512 IFMA, kohë konstante)"),
                it,
                || unsafe { avx.exp(black_box(&a), black_box(&e), &mut out) },
            );
        }
        bench(
            &format!("RSA-{bits} verify-op s^65537 (vartime)"),
            2_000,
            || eng.exp_vartime(black_box(&a), &[65537], &mut out),
        );
    }
}
