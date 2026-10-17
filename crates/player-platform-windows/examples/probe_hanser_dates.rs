use lofty::config::ParseOptions;
use lofty::file::{AudioFile, FileType, TaggedFile, TaggedFileExt};
use lofty::mpeg::MpegFile;
use lofty::probe::Probe;
use lofty::tag::ItemKey;
use player_core::parse_recording_date;
use std::io::Seek;
use std::path::Path;

fn dump(path: &Path) {
    println!("==== {}", path.display());
    if !path.exists() {
        println!("  missing");
        return;
    }

    let probe = Probe::open(path).expect("open");
    let probe = probe.guess_file_type().expect("guess");
    let file_type = probe.file_type().expect("type");
    println!("  file_type={file_type:?}");

    let tagged: TaggedFile = if file_type == FileType::Mpeg {
        let mut reader = probe.into_inner();
        reader.seek(std::io::SeekFrom::Start(0)).expect("seek");
        let mpeg = MpegFile::read_from(&mut reader, ParseOptions::new()).expect("mpeg");
        TaggedFile::from(mpeg)
    } else {
        probe.read().expect("read")
    };

    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let raw = tag.and_then(|tag| tag.get_string(ItemKey::RecordingDate));
    println!(
        "  RecordingDate raw={raw:?} parsed={:?}",
        raw.and_then(parse_recording_date)
    );
    if let Some(tag) = tag {
        for item in tag.items() {
            let key = format!("{:?}", item.key());
            let low = key.to_lowercase();
            if low.contains("date") || low.contains("year") || low.contains("original") {
                println!("  item {key} = {:?}", item.value());
            }
        }
    }
}

fn main() {
    for path in [
        r"D:\Music\Loss\[hanser]\hanser - 彩色膠片.mp3",
        r"D:\Music\Loss\[hanser]\hanser - 蘇生之羽.mp3",
        r"D:\Music\Loss\[hanser]\hanser - さよならスーヴェニア.mp3",
        r"D:\Music\CD\[CD][hanser]\hanser - モニタリング.flac",
        r"D:\Music\CD\[CD][hanser]\hanser - KING.flac",
    ] {
        dump(Path::new(path));
    }
}
