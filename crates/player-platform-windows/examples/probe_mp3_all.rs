use lofty::file::TaggedFileExt;
use lofty::probe::Probe;
use std::path::Path;
fn main() {
    let path = Path::new(r"D:\Music\Loss\[hanser]\hanser - 彩色膠片.mp3");
    let tagged = Probe::open(path).unwrap().read().unwrap();
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag()).unwrap();
    println!("items={}", tag.items().count());
    for item in tag.items() {
        println!("{:?} = {:?}", item.key(), item.value());
    }
}
