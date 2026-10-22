//! The golden table recorded from the former opencc-js conversion (ADR 0030). Every case must
//! match it, except the entries documented in zh-convert-differences.json, which must match
//! the new value recorded there.

use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    category: String,
    input: String,
    lyrics_traditional: String,
    search_traditional: String,
    search_simplified: String,
}

#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Difference {
    input: String,
    /// `lyricsTraditional`, `searchTraditional` or `searchSimplified`.
    field: String,
    opencc_js: String,
    rust: String,
}

#[derive(Deserialize)]
struct Differences {
    differences: Vec<Difference>,
}

fn load<T: for<'de> Deserialize<'de>>(name: &str) -> T {
    let path = format!("{}/../../tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
}

#[test]
fn rust_reproduces_the_opencc_js_table() {
    let fixture: Fixture = load("zh-convert-opencc-js.json");
    let documented: BTreeMap<(String, String), Difference> =
        load::<Differences>("zh-convert-differences.json")
            .differences
            .into_iter()
            .map(|d| ((d.input.clone(), d.field.clone()), d))
            .collect();

    let mut failures = Vec::new();
    let mut seen = 0;
    for case in &fixture.cases {
        let actual = [
            (
                "lyricsTraditional",
                &case.lyrics_traditional,
                player_zh::to_traditional(&case.input).into_owned(),
            ),
            // Search now uses the same Taiwan-glyph Traditional as display (ADR 0030).
            (
                "searchTraditional",
                &case.search_traditional,
                player_zh::to_traditional(&case.input).into_owned(),
            ),
            (
                "searchSimplified",
                &case.search_simplified,
                player_zh::to_simplified(&case.input).into_owned(),
            ),
        ];
        for (field, expected, actual) in actual {
            match documented.get(&(case.input.clone(), field.to_owned())) {
                Some(difference) => {
                    seen += 1;
                    if difference.opencc_js != *expected || difference.rust != actual {
                        failures.push(format!(
                            "[{}] {field} {:?}: documented {:?}→{:?}, now {expected:?}→{actual:?}",
                            case.category, case.input, difference.opencc_js, difference.rust
                        ));
                    }
                }
                None if *expected != actual => failures.push(format!(
                    "[{}] {field} {:?}: opencc-js {expected:?}, rust {actual:?}",
                    case.category, case.input
                )),
                None => {}
            }
        }
    }
    assert_eq!(
        seen,
        documented.len(),
        "every documented difference must still occur"
    );
    assert!(
        failures.is_empty(),
        "undocumented differences:\n{}",
        failures.join("\n")
    );
}
