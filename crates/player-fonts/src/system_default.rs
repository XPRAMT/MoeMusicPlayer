//! Android's default font (ADR 0042), for the pickers' 「預設（…）」 label: the WebView's and the overlay's default
//! `sans-serif` draws Latin text in the config's `sans-serif` family and CJK text in the fallback family for the
//! system's language. Read from `/system/etc/font_fallback.xml` (Android 15 and later) or `/system/etc/fonts.xml`.

use crate::catalog::FontFamily;
use crate::region::FontRegion;

/// The system font configurations, newest first.
pub const ANDROID_FONT_CONFIGS: [&str; 2] =
    ["/system/etc/font_fallback.xml", "/system/etc/fonts.xml"];

/// A font of the system font configuration: its file name (in `/system/fonts`) and face index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemFontRef {
    pub file: String,
    pub index: u32,
}

/// What the default text draws in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AndroidFontConfig {
    /// The regular face of the first `sans-serif` family.
    pub sans_serif: Option<SystemFontRef>,
    /// Unnamed fallback families: their languages (`lang`, split on spaces and commas) and regular face.
    pub fallbacks: Vec<(Vec<String>, SystemFontRef)>,
}

/// The value of `name="…"` in a tag's attribute text.
fn attribute<'a>(attributes: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = attributes;
    while let Some(at) = rest.find(name) {
        let before_ok = at == 0 || rest.as_bytes()[at - 1].is_ascii_whitespace();
        let after = rest[at + name.len()..].trim_start();
        if before_ok {
            if let Some(value) = after.strip_prefix('=') {
                let value = value.trim_start();
                let quote = value.chars().next()?;
                if quote == '"' || quote == '\'' {
                    let value = &value[1..];
                    return value.find(quote).map(|end| &value[..end]);
                }
            }
        }
        rest = &rest[at + name.len()..];
    }
    None
}

/// The regular upright face of a `<family>` element's content: weight 400 and normal style if listed, else the first
/// font; fonts that only stand in for another family (`fallbackFor`, e.g. serif CJK) are skipped.
fn regular_font(content: &str) -> Option<SystemFontRef> {
    let mut fonts: Vec<(bool, SystemFontRef)> = Vec::new();
    let mut rest = content;
    while let Some(start) = rest.find("<font") {
        let after = &rest[start + 5..];
        let Some(tag_end) = after.find('>') else {
            break;
        };
        let attributes = &after[..tag_end];
        // `<font …/>` has no file; `<fontVariation…>` and the like are other elements.
        if !after.starts_with(|c: char| c.is_ascii_whitespace() || c == '>')
            || attributes.ends_with('/')
        {
            rest = &after[tag_end..];
            continue;
        }
        let body = &after[tag_end + 1..];
        let Some(close) = body.find("</font>") else {
            break;
        };
        // The file name is the text, without nested elements (`<axis …/>`).
        let mut file = String::new();
        let mut inside_tag = false;
        for c in body[..close].chars() {
            match c {
                '<' => inside_tag = true,
                '>' => inside_tag = false,
                _ if !inside_tag => file.push(c),
                _ => {}
            }
        }
        let file = file.trim().to_owned();
        if !file.is_empty() && attribute(attributes, "fallbackFor").is_none() {
            let regular = attribute(attributes, "weight").is_none_or(|weight| weight == "400")
                && attribute(attributes, "style").is_none_or(|style| style == "normal");
            let index = attribute(attributes, "index")
                .and_then(|index| index.parse().ok())
                .unwrap_or(0);
            fonts.push((regular, SystemFontRef { file, index }));
        }
        rest = &body[close + 7..];
    }
    let regular = fonts.iter().position(|(regular, _)| *regular).unwrap_or(0);
    (!fonts.is_empty()).then(|| fonts.swap_remove(regular).1)
}

/// Parse a `fonts.xml` / `font_fallback.xml`.
pub fn parse_android_font_config(xml: &str) -> AndroidFontConfig {
    let mut config = AndroidFontConfig::default();
    let mut rest = xml;
    while let Some(start) = rest.find("<family") {
        let after = &rest[start + 7..];
        let Some(tag_end) = after.find('>') else {
            break;
        };
        let attributes = &after[..tag_end];
        if attributes.ends_with('/')
            || !after.starts_with(|c: char| c.is_ascii_whitespace() || c == '>')
        {
            rest = &after[tag_end..];
            continue;
        }
        let body = &after[tag_end + 1..];
        let close = body.find("</family>").unwrap_or(body.len());
        if let Some(font) = regular_font(&body[..close]) {
            match attribute(attributes, "name") {
                Some("sans-serif") if config.sans_serif.is_none() => config.sans_serif = Some(font),
                Some(_) => {}
                None => {
                    if let Some(lang) = attribute(attributes, "lang") {
                        let langs = lang
                            .split([' ', ','])
                            .filter(|part| !part.is_empty())
                            .map(str::to_owned)
                            .collect();
                        config.fallbacks.push((langs, font));
                    }
                }
            }
        }
        rest = &body[close.min(body.len())..];
    }
    config
}

/// The font that draws the default text's CJK characters for `region` (Taiwan and Hong Kong: `zh-Hant`, China:
/// `zh-Hans`, Japan: `ja`), else the `sans-serif` font.
pub fn android_default_font(
    config: &AndroidFontConfig,
    region: FontRegion,
) -> Option<SystemFontRef> {
    let wanted = match region {
        FontRegion::Taiwan | FontRegion::HongKong => Some("zh-Hant"),
        FontRegion::China => Some("zh-Hans"),
        FontRegion::Japan => Some("ja"),
        FontRegion::Other => None,
    };
    wanted
        .and_then(|lang| {
            config.fallbacks.iter().find_map(|(langs, font)| {
                langs
                    .iter()
                    .any(|candidate| {
                        candidate.eq_ignore_ascii_case(lang)
                            || candidate.starts_with(&format!("{lang}-"))
                    })
                    .then(|| font.clone())
            })
        })
        .or_else(|| config.sans_serif.clone())
}

/// The listed family with a face in file `font.file` (any font folder) at `font.index`.
pub fn family_of_system_font<'a>(
    catalog: &'a [FontFamily],
    font: &SystemFontRef,
) -> Option<&'a FontFamily> {
    catalog.iter().find(|family| {
        family.faces.iter().any(|face| {
            face.face_index == font.index
                && face
                    .path
                    .rsplit(['/', '\\'])
                    .next()
                    .is_some_and(|name| name == font.file)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::FamilyFace;
    use crate::region::FontGroup;

    const FONTS_XML: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<familyset version="23">
    <!-- first font is default -->
    <family name="sans-serif">
        <font weight="100" style="normal">Roboto-Regular.ttf
          <axis tag="wght" stylevalue="100" />
        </font>
        <font weight="400" style="normal">Roboto-Regular.ttf
          <axis tag="wght" stylevalue="400" />
        </font>
        <font weight="400" style="italic">Roboto-Italic.ttf</font>
    </family>
    <alias name="arial" to="sans-serif" />
    <family name="sans-serif-condensed">
        <font weight="400" style="normal">RobotoCondensed-Regular.ttf</font>
    </family>
    <family lang="und-Arab" variant="elegant">
        <font weight="400" style="normal">NotoNaskhArabic-Regular.ttf</font>
    </family>
    <family lang="zh-Hans">
        <font weight="400" style="normal" index="2" postScriptName="NotoSansCJKjp-Regular">NotoSansCJK-Regular.ttc</font>
        <font weight="400" style="normal" index="2" fallbackFor="serif">NotoSerifCJK-Regular.ttc</font>
    </family>
    <family lang="zh-Hant,zh-Bopo">
        <font weight="400" style="normal" index="3">NotoSansCJK-Regular.ttc</font>
    </family>
    <family lang="ja ja-Latn">
        <font weight="400" style="normal" index="0">NotoSansCJK-Regular.ttc</font>
    </family>
</familyset>"#;

    fn font(file: &str, index: u32) -> SystemFontRef {
        SystemFontRef {
            file: file.to_owned(),
            index,
        }
    }

    #[test]
    fn reads_the_sans_serif_family_and_the_cjk_fallbacks() {
        let config = parse_android_font_config(FONTS_XML);
        assert_eq!(
            config.sans_serif,
            Some(font("Roboto-Regular.ttf", 0)),
            "the weight 400 face"
        );
        assert_eq!(config.fallbacks.len(), 4);
        assert_eq!(
            config.fallbacks[2].0,
            vec!["zh-Hant".to_owned(), "zh-Bopo".to_owned()]
        );
        assert_eq!(
            android_default_font(&config, FontRegion::Taiwan),
            Some(font("NotoSansCJK-Regular.ttc", 3))
        );
        assert_eq!(
            android_default_font(&config, FontRegion::HongKong),
            Some(font("NotoSansCJK-Regular.ttc", 3))
        );
        assert_eq!(
            android_default_font(&config, FontRegion::China),
            Some(font("NotoSansCJK-Regular.ttc", 2)),
            "the serif stand-in is skipped"
        );
        assert_eq!(
            android_default_font(&config, FontRegion::Japan),
            Some(font("NotoSansCJK-Regular.ttc", 0))
        );
        assert_eq!(
            android_default_font(&config, FontRegion::Other),
            Some(font("Roboto-Regular.ttf", 0))
        );
    }

    #[test]
    fn a_config_without_fallbacks_or_families_degrades() {
        let config = parse_android_font_config(
            r#"<familyset><family name="sans-serif"><font>A.ttf</font></family></familyset>"#,
        );
        assert_eq!(
            android_default_font(&config, FontRegion::Taiwan),
            Some(font("A.ttf", 0))
        );
        assert_eq!(
            android_default_font(&parse_android_font_config("garbage"), FontRegion::Japan),
            None
        );
    }

    #[test]
    fn the_font_is_matched_to_the_listed_family_by_file_and_face() {
        let family = |name: &str, path: &str, index: u32| FontFamily {
            family: name.to_owned(),
            display_name: name.to_owned(),
            group: FontGroup::Latin,
            hidden: false,
            faces: vec![FamilyFace {
                path: path.to_owned(),
                face_index: index,
                weight: 400,
                italic: false,
            }],
        };
        let catalog = vec![
            family(
                "Noto Sans CJK JP",
                "/system/fonts/NotoSansCJK-Regular.ttc",
                0,
            ),
            family(
                "Noto Sans CJK TC",
                "/system/fonts/NotoSansCJK-Regular.ttc",
                3,
            ),
            family("Roboto", "/system/fonts/Roboto-Regular.ttf", 0),
        ];
        let found = |file, index| {
            family_of_system_font(&catalog, &font(file, index)).map(|f| f.family.as_str())
        };
        assert_eq!(
            found("NotoSansCJK-Regular.ttc", 3),
            Some("Noto Sans CJK TC")
        );
        assert_eq!(found("Roboto-Regular.ttf", 0), Some("Roboto"));
        assert_eq!(found("Roboto-Regular.ttf", 1), None);
        assert_eq!(found("Missing.ttf", 0), None);
    }
}
