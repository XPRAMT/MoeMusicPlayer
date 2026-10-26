//! Diagnostic: parse every installed font and print the picker's list for a region.
//! `cargo run -p player-fonts --example list_fonts -- [REGION]` (default: the system region).

use player_fonts::{build_catalog, read_font_file, system_font_files, system_region, FontRegion};
use std::time::Instant;

fn main() {
    let region_code = std::env::args()
        .nth(1)
        .or_else(system_region)
        .unwrap_or_default();
    let region = FontRegion::from_code(&region_code);
    let started = Instant::now();
    let files = system_font_files();
    let listed = started.elapsed();
    let mut faces = Vec::new();
    let mut failed = 0;
    for path in &files {
        match read_font_file(path) {
            Ok(found) => {
                // A second argument prints the raw names of the faces whose family contains it.
                if let Some(needle) = std::env::args().nth(2) {
                    for face in found.iter().filter(|face| {
                        face.names
                            .family
                            .iter()
                            .any(|name| name.name.contains(&needle))
                    }) {
                        println!("{} #{}: {:?}", path.display(), face.face_index, face.names);
                    }
                }
                faces.extend(
                    found
                        .into_iter()
                        .map(|face| (path.to_string_lossy().into_owned(), face)),
                )
            }
            Err(_) => failed += 1,
        }
    }
    let parsed = started.elapsed();
    let catalog = build_catalog(faces, region, |a, b| a.cmp(b));
    println!(
        "region {region_code:?} ({region:?}): {} files ({failed} unreadable), {} families; listed in {listed:?}, parsed in {parsed:?}",
        files.len(),
        catalog.len()
    );
    // Families the pickers do not offer print as `Hidden`; the last line counts the families of each group.
    let label = |family: &player_fonts::FontFamily| {
        if family.hidden {
            "Hidden".to_owned()
        } else {
            format!("{:?}", family.group)
        }
    };
    let mut counts = std::collections::BTreeMap::new();
    for family in &catalog {
        println!(
            "{}\t{}\t[{}]\t{} face(s)",
            label(family),
            family.display_name,
            family.family,
            family.faces.len()
        );
        *counts.entry(label(family)).or_insert(0) += 1;
    }
    println!("groups: {counts:?}");
}
