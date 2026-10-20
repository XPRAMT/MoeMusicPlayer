//! A real DSD song made into DSD-in-WavPack by the official `wavpack` (`wavpack song.dsf song.wv`, run by hand): the
//! 1-bit stream read from the `.wv` must equal the original DSF's bit for bit (checked over the whole song), and so
//! must the PCM the project's DSD converter makes of it (a stretch from the start, the middle and the end, after
//! exact seeks).
//!
//! `MOE_WV_DSD=song.wv MOE_WV_DSF=song.dsf cargo test -p player-wavpack --release --test real_dsd -- --ignored --nocapture`

use std::io::BufReader;
use std::path::PathBuf;
use std::sync::atomic::AtomicU32;
use std::sync::Arc;
use std::time::Instant;

use player_dsd::dsf::DsfReader;
use player_dsd::reader::DsdPcmReader;
use player_dsd::stream::DsdStream;
use player_wavpack::dsd::WavPackDsdStream;

fn wv() -> PathBuf {
    PathBuf::from(std::env::var("MOE_WV_DSD").expect("MOE_WV_DSD"))
}

fn dsf() -> DsfReader<BufReader<std::fs::File>> {
    let file = std::fs::File::open(std::env::var("MOE_WV_DSF").expect("MOE_WV_DSF")).unwrap();
    DsfReader::open(BufReader::new(file)).unwrap()
}

#[test]
#[ignore = "needs MOE_WV_DSD and MOE_WV_DSF"]
fn the_wavpack_dsd_stream_equals_the_dsf_over_the_whole_song() {
    let mut from_wv = WavPackDsdStream::open_path(&wv()).unwrap();
    let mut from_dsf = dsf();
    let (a, b) = (from_wv.info(), from_dsf.info());
    assert_eq!((a.channels, a.sample_rate), (b.channels, b.sample_rate));
    assert_eq!(a.bytes_per_channel(), b.bytes_per_channel());
    let started = Instant::now();
    let channels = usize::from(a.channels);
    let (mut wv_chunk, mut dsf_chunk) = (vec![Vec::new(); channels], vec![Vec::new(); channels]);
    let mut total = 0_u64;
    loop {
        let n = from_wv.read(65_536, &mut wv_chunk).unwrap();
        let m = from_dsf.read(n.max(1), &mut dsf_chunk).unwrap();
        assert_eq!(n, m, "chunk length at byte {total}");
        if n == 0 {
            break;
        }
        assert_eq!(wv_chunk, dsf_chunk, "1-bit data differs near byte {total}");
        total += n as u64;
    }
    println!(
        "{} bytes per channel ({:.1} s of {:.3} MHz DSD) identical to the DSF in {:.1}s",
        total,
        a.duration().as_secs_f64(),
        f64::from(a.sample_rate) / 1e6,
        started.elapsed().as_secs_f64()
    );
}

#[test]
#[ignore = "needs MOE_WV_DSD and MOE_WV_DSF"]
fn the_converted_pcm_and_seeks_equal_the_dsf() {
    let gain = Arc::new(AtomicU32::new(3.0_f32.to_bits()));
    let mut from_wv = DsdPcmReader::new(
        Box::new(WavPackDsdStream::open_path(&wv()).unwrap()) as Box<dyn DsdStream>,
        176_400,
        Arc::clone(&gain),
    )
    .unwrap();
    let mut from_dsf = DsdPcmReader::new(dsf(), 176_400, gain).unwrap();
    let frames = from_dsf.total_frames();
    assert_eq!(from_wv.total_frames(), frames);
    for fraction in [0.0, 0.31, 0.5, 0.97] {
        let frame = (frames as f64 * fraction) as u64;
        from_wv.seek_to_frame(frame).unwrap();
        from_dsf.seek_to_frame(frame).unwrap();
        let (mut a, mut b) = (Vec::new(), Vec::new());
        // The two streams hand out different chunk sizes, so read until both have a stretch and compare that.
        while a.len() < 200_000 {
            from_wv.read_frames(&mut a).unwrap();
        }
        while b.len() < 200_000 {
            from_dsf.read_frames(&mut b).unwrap();
        }
        let common = a.len().min(b.len());
        assert!(common >= 200_000);
        assert!(
            a[..common]
                .iter()
                .zip(&b[..common])
                .all(|(x, y)| x.to_bits() == y.to_bits()),
            "PCM at {fraction}"
        );
        a.truncate(common);
        let peak = a.iter().fold(0.0_f32, |peak, v| peak.max(v.abs()));
        println!("{fraction}: {} samples equal, peak {peak:.3}", common);
    }
}

#[test]
#[ignore = "needs MOE_WV_DSD and MOE_WV_DSF"]
fn the_dop_words_equal_the_dsf_over_the_whole_song_and_after_seeks() {
    use player_dsd::dop::DopStream;
    let mut from_wv =
        DopStream::new(Box::new(WavPackDsdStream::open_path(&wv()).unwrap()) as Box<dyn DsdStream>)
            .unwrap();
    let mut from_dsf = DopStream::new(dsf()).unwrap();
    assert_eq!(from_wv.transport_rate(), from_dsf.transport_rate());
    assert_eq!(from_wv.total_frames(), from_dsf.total_frames());
    let started = Instant::now();
    let (mut a, mut b) = (Vec::new(), Vec::new());
    let mut total = 0_u64;
    loop {
        a.clear();
        b.clear();
        let n = from_wv.read_frames(4_096, &mut a).unwrap();
        let m = from_dsf.read_frames(4_096, &mut b).unwrap();
        assert_eq!(n, m, "frames at {total}");
        if n == 0 {
            break;
        }
        assert_eq!(a, b, "DoP words differ near frame {total}");
        total += n as u64;
    }
    println!(
        "{total} DoP frames at {} Hz identical to the DSF's in {:.1}s",
        from_wv.transport_rate(),
        started.elapsed().as_secs_f64()
    );
    for fraction in [0.0, 0.123, 0.5, 0.9999] {
        let frame = (from_dsf.total_frames() as f64 * fraction) as u64;
        from_wv.seek_to_frame(frame).unwrap();
        from_dsf.seek_to_frame(frame).unwrap();
        a.clear();
        b.clear();
        from_wv.read_frames(4_096, &mut a).unwrap();
        from_dsf.read_frames(4_096, &mut b).unwrap();
        assert_eq!(a, b, "after a seek to frame {frame}");
    }
}
