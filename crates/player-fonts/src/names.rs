//! Choosing among a font's localized names (ADR 0042).

use crate::region::FontRegion;
use serde::{Deserialize, Serialize};

/// One localized name of a font. `lang` is the Windows LCID of the name record (Macintosh English names are stored as
/// 0x0409, Unicode-platform names as 0).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalizedName {
    pub lang: u16,
    pub name: String,
}

const ZH_TW: u16 = 0x0404; // 1028
const ZH_CN: u16 = 0x0804; // 2052
const ZH_HK: u16 = 0x0C04; // 3076
const ZH_SG: u16 = 0x1004; // 4100
const ZH_MO: u16 = 0x1404; // 5124
const JA_JP: u16 = 0x0411; // 1041
const EN_US: u16 = 0x0409; // 1033

/// The name languages tried first in `region`, best first; after them any English, then any name.
pub fn language_preference(region: FontRegion) -> &'static [u16] {
    match region {
        FontRegion::Taiwan => &[ZH_TW, ZH_HK, ZH_MO, ZH_CN, ZH_SG, EN_US],
        FontRegion::HongKong => &[ZH_HK, ZH_MO, ZH_TW, ZH_CN, ZH_SG, EN_US],
        FontRegion::China => &[ZH_CN, ZH_SG, ZH_TW, ZH_HK, ZH_MO, EN_US],
        FontRegion::Japan => &[JA_JP, EN_US],
        FontRegion::Other => &[EN_US],
    }
}

fn is_english(lang: u16) -> bool {
    lang & 0x03FF == 0x09
}

/// The name shown in `region`: the region's languages in order, else any English name, else a Unicode-platform name,
/// else the first.
pub fn pick_name(names: &[LocalizedName], region: FontRegion) -> Option<&str> {
    language_preference(region)
        .iter()
        .find_map(|&lang| names.iter().find(|name| name.lang == lang))
        .or_else(|| names.iter().find(|name| is_english(name.lang)))
        .or_else(|| names.iter().find(|name| name.lang == 0))
        .or_else(|| names.first())
        .map(|name| name.name.as_str())
}

/// The region-independent name stored in the settings (and given to CSS and DirectWrite, which match every localized
/// name): US English, else any English, else a Unicode-platform name, else the first.
pub fn canonical_name(names: &[LocalizedName]) -> Option<&str> {
    pick_name(names, FontRegion::Other)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[(u16, &str)]) -> Vec<LocalizedName> {
        list.iter()
            .map(|&(lang, name)| LocalizedName {
                lang,
                name: name.to_owned(),
            })
            .collect()
    }

    #[test]
    fn the_region_picks_the_localized_name() {
        let jhenghei = names(&[(EN_US, "Microsoft JhengHei"), (ZH_TW, "微軟正黑體")]);
        assert_eq!(pick_name(&jhenghei, FontRegion::Taiwan), Some("微軟正黑體"));
        // Hong Kong and China fall back to the Taiwan name before English.
        assert_eq!(
            pick_name(&jhenghei, FontRegion::HongKong),
            Some("微軟正黑體")
        );
        assert_eq!(pick_name(&jhenghei, FontRegion::China), Some("微軟正黑體"));
        // Japan and others show English rather than a Chinese name.
        assert_eq!(
            pick_name(&jhenghei, FontRegion::Japan),
            Some("Microsoft JhengHei")
        );
        assert_eq!(
            pick_name(&jhenghei, FontRegion::Other),
            Some("Microsoft JhengHei")
        );
        assert_eq!(canonical_name(&jhenghei), Some("Microsoft JhengHei"));

        let hk_and_cn = names(&[(ZH_CN, "简"), (ZH_HK, "港"), (JA_JP, "日")]);
        assert_eq!(pick_name(&hk_and_cn, FontRegion::Taiwan), Some("港"));
        assert_eq!(pick_name(&hk_and_cn, FontRegion::China), Some("简"));
        assert_eq!(pick_name(&hk_and_cn, FontRegion::Japan), Some("日"));
        // No English: the first name.
        assert_eq!(canonical_name(&hk_and_cn), Some("简"));
    }

    #[test]
    fn english_variants_and_unicode_names_are_fallbacks() {
        let british = names(&[(0x0809, "Colour Sans"), (ZH_TW, "彩")]);
        assert_eq!(pick_name(&british, FontRegion::Japan), Some("Colour Sans"));
        assert_eq!(canonical_name(&british), Some("Colour Sans"));
        let unicode = names(&[(0x0412, "한글"), (0, "Plain")]);
        assert_eq!(canonical_name(&unicode), Some("Plain"));
        assert_eq!(pick_name(&[], FontRegion::Taiwan), None);
    }
}
