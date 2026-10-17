use lofty::config::ParseOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::id3::v2::FrameId;
use lofty::mpeg::MpegFile;
use lofty::prelude::Accessor;
use lofty::probe::Probe;
use lofty::tag::ItemKey;
use std::borrow::Cow;
use std::path::Path;

fn main() {
    let path = Path::new(r"D:\Music\Loss\[hanser]\hanser - 彩色膠片.mp3");
    let mut file = std::fs::File::open(path).unwrap();
    let mpeg = MpegFile::read_from(&mut file, ParseOptions::new()).unwrap();
    println!("has id3v2={}", mpeg.id3v2().is_some());
    if let Some(id3) = mpeg.id3v2() {
        println!("Id3v2Tag date()={:?}", Accessor::date(id3));
        println!("len={}", id3.len());
        let tdrc = FrameId::Valid(Cow::Borrowed("TDRC"));
        println!("get TDRC={:?}", id3.get(&tdrc).map(|f| format!("{f:?}")));
        let tyer = FrameId::Valid(Cow::Borrowed("TYER"));
        println!("get TYER={:?}", id3.get(&tyer).map(|f| format!("{f:?}")));
    }

    let tagged = Probe::open(path).unwrap().read().unwrap();
    let tag = tagged.primary_tag().unwrap();
    println!(
        "primary Tag RecordingDate={:?}",
        tag.get_string(ItemKey::RecordingDate)
    );
    println!("primary Tag date()={:?}", Accessor::date(tag));
}
