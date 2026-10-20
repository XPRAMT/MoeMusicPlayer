//! Writes the synthetic DSD64 file (a 1 kHz and a 2 kHz tone, 0.2 s) that `fixtures/dsd64.wv` was made from:
//! `MOE_DSD_OUT=x.dsf cargo test -p player-wavpack --test make_dsd_fixture -- --ignored`, then
//! `wavpack -y -m x.dsf dsd64.wv` and `wvunpack -y --raw-pcm dsd64.wv` (the 24-bit decimation, `dsd64.raw24`).

use player_dsd::testing::{build_dsf, modulate, tone};

#[test]
#[ignore = "writes the fixture source file"]
fn write_the_synthetic_dsf() {
    let rate = 2_822_400_u32;
    let samples = rate as usize / 5 / 16 * 16;
    let lsb = |bytes: Vec<u8>| bytes.into_iter().map(u8::reverse_bits).collect::<Vec<u8>>();
    let left = lsb(modulate(samples, tone(rate, 1_000.0, 0.25)));
    let right = lsb(modulate(samples, tone(rate, 2_000.0, 0.25)));
    std::fs::write(
        std::env::var("MOE_DSD_OUT").unwrap(),
        build_dsf(rate, samples as u64, 4096, true, &[left, right]),
    )
    .unwrap();
}
