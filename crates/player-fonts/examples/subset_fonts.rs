//! Diagnostic: make the name subset (ADR 0042) of every listed family of this system and report sizes and time.
//! `cargo run --release -p player-fonts --example subset_fonts -- OUT_DIR [REGION]`

use player_fonts::{
    build_catalog, name_subset, read_font_file, read_standalone_face, system_font_files,
    system_region, visible_families, FontRegion,
};
use std::time::Instant;

fn main() {
    let out = std::path::PathBuf::from(std::env::args().nth(1).expect("OUT_DIR"));
    std::fs::create_dir_all(&out).expect("OUT_DIR");
    let region = FontRegion::from_code(
        &std::env::args()
            .nth(2)
            .or_else(system_region)
            .unwrap_or_default(),
    );
    let mut faces = Vec::new();
    for path in system_font_files() {
        if let Ok(found) = read_font_file(&path) {
            faces.extend(
                found
                    .into_iter()
                    .map(|face| (path.to_string_lossy().into_owned(), face)),
            );
        }
    }
    let families = visible_families(&build_catalog(faces, region, |a, b| a.cmp(b)));
    let started = Instant::now();
    // OUT_DIR/index.txt: which file is which family; OUT_DIR/fonts.json: the list as `fonts_list` sends it, with
    // each subset's file (to check the subsets and the picker in a browser).
    let mut listing = String::new();
    let mut listed = Vec::new();
    let (mut made, mut missing, mut total, mut largest) =
        (0usize, Vec::new(), 0usize, (0usize, String::new()));
    for (index, family) in families.iter().enumerate() {
        let face = &family.faces[0];
        let subset = read_standalone_face(std::path::Path::new(&face.path), face.face_index)
            .ok()
            .and_then(|data| name_subset(&data, 0, &family.display_name));
        let mut entry = serde_json::to_value(family).expect("json");
        match subset {
            Some(bytes) => {
                entry["subset"] = out
                    .join(format!("{index:04}.font"))
                    .to_string_lossy()
                    .into();
                made += 1;
                total += bytes.len();
                if bytes.len() > largest.0 {
                    largest = (bytes.len(), family.display_name.clone());
                }
                std::fs::write(out.join(format!("{index:04}.font")), &bytes).expect("write");
                listing.push_str(&format!(
                    "{index:04}\t{}\t{}#{}\n",
                    family.display_name, face.path, face.face_index
                ));
            }
            None => missing.push(family.display_name.clone()),
        }
        listed.push(entry);
    }
    std::fs::write(
        out.join("fonts.json"),
        serde_json::to_string(&listed).expect("json"),
    )
    .expect("fonts.json");
    std::fs::write(out.join("index.txt"), listing).expect("index.txt");
    println!(
        "{} families: {made} subsets, {} KB in all ({} B on average, largest {} B: {}), {} without ({missing:?}); {:?}",
        families.len(),
        total / 1024,
        total / made.max(1),
        largest.0,
        largest.1,
        missing.len(),
        started.elapsed()
    );
}
