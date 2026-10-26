//! The picker's font list (ADR 0042): faces merged into families, grouped by the region's order and sorted by the
//! caller's collation within a group.

use crate::names::{canonical_name, pick_name, LocalizedName};
use crate::parse::FaceInfo;
use crate::region::{font_group, group_rank, FontGroup, FontRegion};
use crate::scripts::Scripts;
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::HashMap;

/// One face of a family: where it lives (Android `@font-face` and Typeface need the file).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FamilyFace {
    pub path: String,
    pub face_index: u32,
    pub weight: u16,
    pub italic: bool,
}

/// One picker entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontFamily {
    /// The stored setting value and the name given to CSS / DirectWrite (`canonical_name`).
    pub family: String,
    /// The name shown, in the region's language (`pick_name`).
    pub display_name: String,
    pub group: FontGroup,
    /// Regular upright face first, then by weight.
    pub faces: Vec<FamilyFace>,
}

/// How far a face is from the regular upright one (the face a family is represented by).
fn regular_distance(face: &FamilyFace) -> (bool, u16, u16) {
    (face.italic, face.weight.abs_diff(400), face.weight)
}

/// Merge `faces` (font file path, face) into families and order them for `region`: groups in the region's order,
/// then `compare` on the shown names (the app's locale-aware natural collation), then the stored name.
pub fn build_catalog(
    faces: impl IntoIterator<Item = (String, FaceInfo)>,
    region: FontRegion,
    compare: impl Fn(&str, &str) -> Ordering,
) -> Vec<FontFamily> {
    struct Building {
        family: String,
        names: Vec<LocalizedName>,
        scripts: Scripts,
        faces: Vec<(FamilyFace, Vec<LocalizedName>)>,
    }
    let mut by_key: HashMap<String, usize> = HashMap::new();
    let mut building: Vec<Building> = Vec::new();
    for (path, face) in faces {
        let Some(family) = canonical_name(&face.names.family).map(str::to_owned) else {
            continue;
        };
        // Vertical "@" families are GDI's, not real families.
        if family.starts_with('@') {
            continue;
        }
        let key = family.to_lowercase();
        let index = *by_key.entry(key).or_insert_with(|| {
            building.push(Building {
                family: family.clone(),
                names: Vec::new(),
                scripts: Scripts::default(),
                faces: Vec::new(),
            });
            building.len() - 1
        });
        let entry = &mut building[index];
        entry.scripts = entry.scripts.union(face.scripts);
        let member = FamilyFace {
            path,
            face_index: face.face_index,
            weight: face.weight,
            italic: face.italic,
        };
        if !entry.faces.iter().any(|(existing, _)| {
            existing.path == member.path && existing.face_index == member.face_index
        }) {
            entry.faces.push((member, face.names.family));
        }
    }
    let mut families: Vec<(usize, FontFamily)> = building
        .into_iter()
        .map(|mut entry| {
            entry.faces.sort_by(|a, b| {
                regular_distance(&a.0)
                    .cmp(&regular_distance(&b.0))
                    .then(a.0.path.cmp(&b.0.path))
            });
            // The regular face's localized names first; other faces add languages it lacks.
            for (_, names) in &entry.faces {
                for name in names {
                    if !entry
                        .names
                        .iter()
                        .any(|existing| existing.lang == name.lang)
                    {
                        entry.names.push(name.clone());
                    }
                }
            }
            let display_name = pick_name(&entry.names, region)
                .unwrap_or(&entry.family)
                .to_owned();
            let group = font_group(entry.scripts, region);
            (
                group_rank(group, region),
                FontFamily {
                    family: entry.family,
                    display_name,
                    group,
                    faces: entry.faces.into_iter().map(|(face, _)| face).collect(),
                },
            )
        })
        .collect();
    families.sort_by(|(rank_a, a), (rank_b, b)| {
        rank_a
            .cmp(rank_b)
            .then_with(|| compare(&a.display_name, &b.display_name))
            .then_with(|| a.family.cmp(&b.family))
    });
    families.into_iter().map(|(_, family)| family).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::FaceNames;

    fn face(
        index: u32,
        family: &[(u16, &str)],
        weight: u16,
        italic: bool,
        scripts: u8,
    ) -> FaceInfo {
        FaceInfo {
            face_index: index,
            names: FaceNames {
                family: family
                    .iter()
                    .map(|&(lang, name)| LocalizedName {
                        lang,
                        name: name.to_owned(),
                    })
                    .collect(),
                ..FaceNames::default()
            },
            weight,
            italic,
            scripts: Scripts(scripts),
        }
    }

    fn sample() -> Vec<(String, FaceInfo)> {
        const T: u8 = Scripts::TRADITIONAL;
        const S: u8 = Scripts::SIMPLIFIED;
        const J: u8 = Scripts::JAPANESE;
        vec![
            (
                "msjhbd.ttc".into(),
                face(
                    0,
                    &[(0x0409, "Microsoft JhengHei"), (0x0404, "微軟正黑體")],
                    700,
                    false,
                    T | J,
                ),
            ),
            (
                "msjh.ttc".into(),
                face(
                    0,
                    &[(0x0409, "Microsoft JhengHei"), (0x0404, "微軟正黑體")],
                    400,
                    false,
                    T | J,
                ),
            ),
            (
                "msyh.ttc".into(),
                face(
                    0,
                    &[(0x0409, "Microsoft YaHei"), (0x0804, "微软雅黑")],
                    400,
                    false,
                    T | S | J,
                ),
            ),
            (
                "simsun.ttc".into(),
                face(0, &[(0x0409, "SimSun"), (0x0804, "宋体")], 400, false, S),
            ),
            (
                "yugoth.ttc".into(),
                face(
                    0,
                    &[(0x0409, "Yu Gothic"), (0x0411, "游ゴシック")],
                    400,
                    false,
                    J,
                ),
            ),
            (
                "arial.ttf".into(),
                face(0, &[(0x0409, "Arial")], 400, false, 0),
            ),
            (
                "ariali.ttf".into(),
                face(0, &[(0x0409, "Arial")], 400, true, 0),
            ),
            (
                "consola.ttf".into(),
                face(0, &[(0x0409, "Consolas")], 400, false, 0),
            ),
            (
                "vert.ttf".into(),
                face(0, &[(0x0409, "@MingLiU")], 400, false, T),
            ),
        ]
    }

    fn order(region: FontRegion) -> Vec<String> {
        build_catalog(sample(), region, |a, b| a.cmp(b))
            .into_iter()
            .map(|family| family.display_name)
            .collect()
    }

    #[test]
    fn styles_merge_into_one_family_with_the_regular_face_first() {
        let catalog = build_catalog(sample(), FontRegion::Taiwan, |a, b| a.cmp(b));
        let jhenghei = catalog
            .iter()
            .find(|f| f.family == "Microsoft JhengHei")
            .unwrap();
        assert_eq!(jhenghei.display_name, "微軟正黑體");
        assert_eq!(jhenghei.faces.len(), 2);
        assert_eq!(jhenghei.faces[0].path, "msjh.ttc");
        let arial = catalog.iter().find(|f| f.family == "Arial").unwrap();
        assert_eq!(
            arial.faces.iter().map(|f| f.italic).collect::<Vec<_>>(),
            vec![false, true]
        );
        assert!(catalog.iter().all(|f| !f.family.starts_with('@')));
        assert_eq!(catalog.len(), 6);
    }

    #[test]
    fn groups_follow_the_region_order() {
        // Code point order stands in for the app's collation; within a group the names are sorted by it.
        assert_eq!(
            order(FontRegion::Taiwan),
            [
                "微軟正黑體",
                "微软雅黑",
                "宋体",
                "Yu Gothic",
                "Arial",
                "Consolas"
            ]
        );
        assert_eq!(
            order(FontRegion::China),
            [
                "宋体",
                "微软雅黑",
                "微軟正黑體",
                "Yu Gothic",
                "Arial",
                "Consolas"
            ]
        );
        assert_eq!(
            order(FontRegion::Japan),
            [
                "Microsoft JhengHei",
                "Microsoft YaHei",
                "游ゴシック",
                "SimSun",
                "Arial",
                "Consolas"
            ]
        );
        assert_eq!(
            order(FontRegion::Other),
            [
                "Arial",
                "Consolas",
                "Microsoft JhengHei",
                "Microsoft YaHei",
                "SimSun",
                "Yu Gothic"
            ]
        );
    }
}
