//! The OS region decides the font list's group order and which localized name each font shows (ADR 0042). It is the
//! system's region, not the app's interface language.

use crate::scripts::Scripts;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontRegion {
    /// TW.
    Taiwan,
    /// HK, MO.
    HongKong,
    /// CN, SG.
    China,
    /// JP.
    Japan,
    /// English-speaking and every other region.
    Other,
}

/// A group of the font list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FontGroup {
    Traditional,
    Simplified,
    Japanese,
    /// No Chinese or Japanese coverage (「只有英文」).
    Latin,
}

impl FontRegion {
    /// From an ISO 3166 region code (`TW`, Windows `GetUserDefaultGeoName`, Android `Locale.getCountry`) or a locale
    /// tag carrying one (`zh-Hant-TW`, `ja_JP`); a tag without a region falls back to its language and script.
    pub fn from_code(code: &str) -> FontRegion {
        let parts: Vec<&str> = code
            .split(['-', '_'])
            .filter(|part| !part.is_empty())
            .collect();
        let region = match parts.as_slice() {
            // A lone upper-case pair is a region code; a lone lower-case word is a language ("ja").
            [single] if single.len() == 2 && single.chars().all(|c| c.is_ascii_uppercase()) => {
                Some((*single).to_owned())
            }
            [_] => None,
            [_, rest @ ..] => rest
                .iter()
                .find(|part| part.len() == 2 && part.chars().all(|c| c.is_ascii_alphabetic()))
                .map(|part| part.to_ascii_uppercase()),
            [] => None,
        };
        match region.as_deref() {
            Some("TW") => return FontRegion::Taiwan,
            Some("HK" | "MO") => return FontRegion::HongKong,
            Some("CN" | "SG") => return FontRegion::China,
            Some("JP") => return FontRegion::Japan,
            Some(_) => return FontRegion::Other,
            None => {}
        }
        let language = parts.first().map(|part| part.to_ascii_lowercase());
        let hant = parts.iter().any(|part| part.eq_ignore_ascii_case("hant"));
        match language.as_deref() {
            Some("zh") if hant => FontRegion::Taiwan,
            Some("zh") => FontRegion::China,
            Some("ja") => FontRegion::Japan,
            _ => FontRegion::Other,
        }
    }
}

/// The group order of the font list in `region`.
pub fn group_order(region: FontRegion) -> [FontGroup; 4] {
    use FontGroup::*;
    match region {
        FontRegion::Taiwan | FontRegion::HongKong => [Traditional, Simplified, Japanese, Latin],
        FontRegion::China => [Simplified, Traditional, Japanese, Latin],
        FontRegion::Japan => [Japanese, Traditional, Simplified, Latin],
        FontRegion::Other => [Latin, Traditional, Simplified, Japanese],
    }
}

/// The group of a font covering `scripts`: the earliest group of the region's order the font supports (a font with
/// no Chinese or Japanese coverage is Latin only).
pub fn font_group(scripts: Scripts, region: FontRegion) -> FontGroup {
    let supports = |group: FontGroup| match group {
        FontGroup::Traditional => scripts.traditional(),
        FontGroup::Simplified => scripts.simplified(),
        FontGroup::Japanese => scripts.japanese(),
        FontGroup::Latin => scripts.0 == 0,
    };
    group_order(region)
        .into_iter()
        .find(|&group| supports(group))
        .unwrap_or(FontGroup::Latin)
}

/// The position of `group` in the region's order.
pub fn group_rank(group: FontGroup, region: FontRegion) -> usize {
    group_order(region)
        .iter()
        .position(|&g| g == group)
        .unwrap_or(usize::MAX)
}

/// This system's region code (Windows: `GetUserDefaultGeoName`, else the region of `GetUserDefaultLocaleName`). On
/// Android the region comes from the plugin (`Locale.getDefault().getCountry()`); elsewhere `LANG`.
pub fn system_region() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Globalization::{GetUserDefaultGeoName, GetUserDefaultLocaleName};
        let mut buffer = [0u16; 85];
        // Both return the length including the terminating null, or 0 on failure.
        let length = unsafe { GetUserDefaultGeoName(&mut buffer) };
        if length > 1 {
            let geo = String::from_utf16_lossy(&buffer[..(length - 1) as usize]);
            if geo.len() == 2 && geo.chars().all(|c| c.is_ascii_alphabetic()) {
                return Some(geo);
            }
        }
        let length = unsafe { GetUserDefaultLocaleName(&mut buffer) };
        if length > 1 {
            return Some(String::from_utf16_lossy(&buffer[..(length - 1) as usize]));
        }
        None
    }
    #[cfg(not(target_os = "windows"))]
    {
        ["LC_ALL", "LANG"]
            .into_iter()
            .find_map(|key| std::env::var(key).ok().filter(|value| !value.is_empty()))
            .map(|value| value.split('.').next().unwrap_or_default().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use FontGroup::*;

    #[test]
    fn region_codes_and_locale_tags() {
        assert_eq!(FontRegion::from_code("TW"), FontRegion::Taiwan);
        assert_eq!(FontRegion::from_code("zh-Hant-TW"), FontRegion::Taiwan);
        assert_eq!(FontRegion::from_code("zh_TW"), FontRegion::Taiwan);
        assert_eq!(FontRegion::from_code("HK"), FontRegion::HongKong);
        assert_eq!(FontRegion::from_code("MO"), FontRegion::HongKong);
        assert_eq!(FontRegion::from_code("zh-CN"), FontRegion::China);
        assert_eq!(FontRegion::from_code("SG"), FontRegion::China);
        assert_eq!(FontRegion::from_code("ja-JP"), FontRegion::Japan);
        assert_eq!(FontRegion::from_code("US"), FontRegion::Other);
        assert_eq!(FontRegion::from_code("en-GB"), FontRegion::Other);
        // A region wins over the language: Japanese in Taiwan orders like Taiwan.
        assert_eq!(FontRegion::from_code("ja-TW"), FontRegion::Taiwan);
        // No region: by language and script.
        assert_eq!(FontRegion::from_code("zh-Hant"), FontRegion::Taiwan);
        assert_eq!(FontRegion::from_code("zh"), FontRegion::China);
        assert_eq!(FontRegion::from_code("ja"), FontRegion::Japan);
        assert_eq!(FontRegion::from_code("en"), FontRegion::Other);
        assert_eq!(FontRegion::from_code(""), FontRegion::Other);
    }

    #[test]
    fn group_orders_follow_the_region() {
        assert_eq!(
            group_order(FontRegion::Taiwan),
            [Traditional, Simplified, Japanese, Latin]
        );
        assert_eq!(
            group_order(FontRegion::HongKong),
            group_order(FontRegion::Taiwan)
        );
        assert_eq!(
            group_order(FontRegion::China),
            [Simplified, Traditional, Japanese, Latin]
        );
        assert_eq!(
            group_order(FontRegion::Japan),
            [Japanese, Traditional, Simplified, Latin]
        );
        assert_eq!(
            group_order(FontRegion::Other),
            [Latin, Traditional, Simplified, Japanese]
        );
    }

    #[test]
    fn a_font_goes_to_its_earliest_supported_group() {
        let all = Scripts(Scripts::TRADITIONAL | Scripts::SIMPLIFIED | Scripts::JAPANESE);
        assert_eq!(font_group(all, FontRegion::Taiwan), Traditional);
        assert_eq!(font_group(all, FontRegion::China), Simplified);
        assert_eq!(font_group(all, FontRegion::Japan), Japanese);
        assert_eq!(font_group(all, FontRegion::Other), Traditional);
        let simplified_and_kana = Scripts(Scripts::SIMPLIFIED | Scripts::JAPANESE);
        assert_eq!(
            font_group(simplified_and_kana, FontRegion::Taiwan),
            Simplified
        );
        assert_eq!(font_group(simplified_and_kana, FontRegion::Japan), Japanese);
        assert_eq!(
            font_group(Scripts(Scripts::JAPANESE), FontRegion::Other),
            Japanese
        );
        assert_eq!(font_group(Scripts(0), FontRegion::Taiwan), Latin);
        assert_eq!(font_group(Scripts(0), FontRegion::Other), Latin);
        assert_eq!(group_rank(Latin, FontRegion::Other), 0);
        assert_eq!(group_rank(Latin, FontRegion::Taiwan), 3);
    }
}
