//! Verify lofty write support on copies of sample files (never the originals).
//! Usage: cargo run -p player-tags --example verify_lofty_write -- <dir-with-sample.*>
use std::path::Path;
use std::sync::atomic::AtomicBool;

use player_tags::{read_details, render_spectrogram, write_edit, SpectrogramOptions, TagEdit};

fn main() {
    let dir = std::env::args().nth(1).expect("dir");
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.starts_with("sample.") {
            continue;
        }
        let work = Path::new(&dir).join(format!("work-{name}"));
        std::fs::copy(&path, &work).unwrap();
        print!("{name}: ");
        let before = match read_details(&work) {
            Ok(d) => d,
            Err(e) => {
                println!("READ FAIL {e}");
                continue;
            }
        };
        print!("tag={} ro={} ", before.tag_format, before.read_only);
        let t0 = std::time::Instant::now();
        match render_spectrogram(
            &work,
            SpectrogramOptions::default(),
            &AtomicBool::new(false),
        ) {
            Ok(s) => print!(
                "spec={}x{} {}ms cutoff={:?} ",
                s.columns,
                s.height,
                t0.elapsed().as_millis(),
                s.cutoff_hz
            ),
            Err(e) => print!("spec ERR {e} "),
        }
        if before.read_only {
            println!();
            continue;
        }
        let mut edit = TagEdit {
            fields: before.fields.clone(),
            ..Default::default()
        };
        edit.fields.title = "測試標題 Title".into();
        edit.fields.artist = "歌手A; 歌手B feat. C".into();
        edit.fields.composer = "作曲者".into();
        edit.fields.lyrics = "[00:01.00]第一行\n[00:02.00]第二行".into();
        edit.fields.date = "2024-05-06".into();
        let mut img = Vec::new();
        image::DynamicImage::new_rgb8(1200, 800)
            .write_to(&mut std::io::Cursor::new(&mut img), image::ImageFormat::Png)
            .unwrap();
        edit.replace_cover = Some(img);
        edit.cover_options = player_tags::CoverOptions {
            jpeg_quality: 85,
            auto_resize: true,
            short_edge_px: 500,
        };
        match write_edit(&work, &edit) {
            Ok(after) => {
                let ok = after.fields == edit.fields;
                println!(
                    "WRITE ok fields_roundtrip={ok} tag={} cover={:?}",
                    after.tag_format,
                    after.cover.map(|c| (c.mime, c.width, c.height))
                );
                if !ok {
                    println!("  want {:?}\n  got  {:?}", edit.fields, after.fields);
                }
            }
            Err(e) => println!("WRITE FAIL {e}"),
        }
    }
    // ID3v2.3 -> v2.4 upgrade check (decision D3): downgrade a copy to v2.3, then edit it.
    let src = Path::new(&dir).join("sample.mp3");
    let v23 = Path::new(&dir).join("work-made-v23.mp3");
    std::fs::copy(&src, &v23).unwrap();
    {
        use lofty::config::WriteOptions;
        use lofty::file::TaggedFileExt;
        use lofty::prelude::TagExt;
        let file = lofty::read_from_path(&v23).unwrap();
        let mut options = WriteOptions::default();
        options.use_id3v23(true);
        file.primary_tag()
            .unwrap()
            .save_to_path(&v23, options)
            .unwrap();
    }
    let before = read_details(&v23).unwrap();
    let mut edit = TagEdit {
        fields: before.fields.clone(),
        ..Default::default()
    };
    edit.fields.title = "v23 upgrade".into();
    let after = write_edit(&v23, &edit).unwrap();
    println!(
        "v2.3 upgrade: before={} after={} title={}",
        before.tag_format, after.tag_format, after.fields.title
    );
}
