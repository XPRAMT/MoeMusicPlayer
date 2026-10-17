use lofty::file::TaggedFileExt;
use lofty::probe::Probe;
use lofty::tag::ItemKey;
use std::path::Path;
fn main() {
    for path in [
        r"D:\Music\Loss\[hanser]\hanser - 彩色膠片.mp3",
        r"D:\Music\Loss\[hanser]\hanser - 蘇生之羽.mp3",
    ] {
        let path = Path::new(path);
        let tagged = Probe::open(path).unwrap().read().unwrap();
        let tag = tagged.primary_tag().or_else(|| tagged.first_tag()).unwrap();
        println!(
            "{} RecordingDate={:?} tags={}",
            path.file_name().unwrap().to_string_lossy(),
            tag.get_string(ItemKey::RecordingDate),
            tagged.tags().len()
        );
        for t in tagged.tags() {
            println!("  tag type {:?} items={}", t.tag_type(), t.items().count());
            for item in t.items() {
                let k = format!("{:?}", item.key());
                if k.to_lowercase().contains("date")
                    || k.to_lowercase().contains("year")
                    || k.contains("Title")
                {
                    println!("    {k}={:?}", item.value());
                }
            }
        }
    }
}
