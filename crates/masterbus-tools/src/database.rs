//! Bundled per-model path suggestions, keyed on **article number** and
//! **field id**.
//!
//! This is the tier the name heuristics in [`crate::seed`] cannot reach.
//! Two charger articles on the bus in issue #6 both advertise as class `CHG`,
//! both present a group called `Output`, and their field sets are unrelated;
//! one of them has had its output fields renamed by the installer. No table
//! keyed on class and field name can tell them apart. Keyed on the article, the
//! ambiguity is gone: the article number is what the firmware reports and what
//! Mastervolt assigns per model.
//!
//! # These are suggestions
//!
//! Nothing here is applied automatically to a curated mapping. The entries back
//! the `+` key in `masterbus-tui --mapping`, where a human sees the proposed
//! path and the conversion it implies before accepting it, and they seed a
//! first `mapping.json`. A wrong entry costs a keystroke, not a wrong reading
//! on a dashboard.
//!
//! Every entry records where it came from, because "one boat reported this"
//! is genuinely different evidence from "this is the vendor's documented
//! meaning", and the difference should survive into review.
//!
//! # Adding a model
//!
//! Run `masterbus-dump --values all --menus all` on a bus with the device, and
//! add an `articles` entry to `suggestions/catalog.json` mapping field ids to
//! paths. `{instance}` is replaced with the device's Signal K instance.

use std::collections::HashMap;
use std::sync::OnceLock;

use masterbus::FieldId;
use serde::Deserialize;

use crate::mapping::parse_field_key;
use crate::seed::Suggestion;

/// The bundled catalog.
const CATALOG_JSON: &str = include_str!("suggestions/catalog.json");

#[derive(Debug, Deserialize)]
struct Catalog {
    articles: HashMap<String, Model>,
}

/// One model's suggestions.
#[derive(Debug, Deserialize)]
struct Model {
    /// Human-readable model name, for the TUI to show alongside a suggestion.
    #[serde(default)]
    model: String,
    /// Where these entries came from. Provenance, not decoration.
    #[serde(default)]
    source: String,
    /// Field id text → entry, for any firmware.
    #[serde(default)]
    fields: HashMap<String, Entry>,
    /// Per-firmware overrides, consulted before `fields`. Field ids can move
    /// between firmware revisions, so a model that is known to have done so
    /// gets an exact entry here.
    #[serde(default)]
    firmware: HashMap<String, HashMap<String, Entry>>,
}

#[derive(Debug, Clone, Deserialize)]
struct Entry {
    /// Signal K path, with `{instance}` still in it.
    path: String,
    /// Publish the boolean negated.
    #[serde(default)]
    invert: bool,
}

/// Parsed catalog, indexed by article and then by field id.
struct Index {
    models: HashMap<String, ModelIndex>,
}

struct ModelIndex {
    model: String,
    source: String,
    any: HashMap<FieldId, Entry>,
    by_firmware: HashMap<String, HashMap<FieldId, Entry>>,
}

fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| {
        let parsed: Catalog = serde_json::from_str(CATALOG_JSON)
            .expect("bundled suggestion catalog must parse; it is checked by a test");
        let convert = |m: HashMap<String, Entry>| -> HashMap<FieldId, Entry> {
            m.into_iter()
                .filter_map(|(k, v)| parse_field_key(&k).map(|id| (id, v)))
                .collect()
        };
        Index {
            models: parsed
                .articles
                .into_iter()
                .map(|(article, m)| {
                    (
                        article,
                        ModelIndex {
                            model: m.model,
                            source: m.source,
                            any: convert(m.fields),
                            by_firmware: m
                                .firmware
                                .into_iter()
                                .map(|(fw, f)| (fw, convert(f)))
                                .collect(),
                        },
                    )
                })
                .collect(),
        }
    })
}

/// A suggestion together with where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Known {
    /// The proposed mapping, `{instance}` already substituted.
    pub suggestion: Suggestion,
    /// The model name this entry belongs to.
    pub model: String,
    /// Provenance of the entry.
    pub source: String,
    /// Whether the entry was an exact match on this firmware, rather than the
    /// model's firmware-independent list.
    pub exact_firmware: bool,
}

/// Look up a field by the device's article and firmware.
///
/// A firmware-specific entry wins; otherwise the model's firmware-independent
/// list is used. `None` when the model is not bundled, which is the common case
/// and not an error.
pub fn lookup(article: &str, firmware: &str, field: FieldId, instance: &str) -> Option<Known> {
    let m = index().models.get(article.trim())?;
    let (entry, exact) = m
        .by_firmware
        .get(firmware.trim())
        .and_then(|f| f.get(&field))
        .map(|e| (e, true))
        .or_else(|| m.any.get(&field).map(|e| (e, false)))?;
    Some(Known {
        suggestion: Suggestion {
            path: entry.path.replace("{instance}", instance),
            invert: entry.invert,
        },
        model: m.model.clone(),
        source: m.source.clone(),
        exact_firmware: exact,
    })
}

/// Whether any suggestions are bundled for an article.
pub fn knows_article(article: &str) -> bool {
    index().models.contains_key(article.trim())
}

/// Article numbers the catalog covers, sorted. For the TUI's status line and
/// for tests.
pub fn articles() -> Vec<&'static str> {
    let mut v: Vec<&str> = index().models.keys().map(|s| s.as_str()).collect();
    v.sort_unstable();
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signalk::leaf_unit;
    use crate::units::conversion;

    #[test]
    fn the_bundled_catalog_parses() {
        assert!(!articles().is_empty());
    }

    /// The case that motivated the whole redesign: two articles, both class
    /// `CHG`, both with a group called `Output`, whose field `0x00E` exists on
    /// one and not the other.
    #[test]
    fn the_two_charger_articles_are_told_apart() {
        let mass = lookup("40021006", "7.9", 0x00E, "ch1").expect("Mass has 0x00E");
        assert_eq!(mass.suggestion.path, "electrical.chargers.ch1.voltage");
        // The ChargeMaster has no 0x00E; its outputs live elsewhere.
        assert!(lookup("44010250", "0.5", 0x00E, "ch1").is_none());
        let cm = lookup("44010250", "0.5", 0x002, "ch1").expect("ChargeMaster has 0x002");
        assert_eq!(
            cm.suggestion.path,
            "electrical.chargers.ch1.output.1.voltage"
        );
    }

    /// The renamed-field case from #6: `0x002` and `0x004` were relabelled
    /// `Eng.batt` and `Gen.batt` on that boat. The lookup never sees a name.
    #[test]
    fn renamed_fields_are_reached_by_id() {
        for (id, out) in [(0x002u16, "1"), (0x004, "2"), (0x006, "3")] {
            let k = lookup("44010250", "0.5", id, "chargere").unwrap();
            assert_eq!(
                k.suggestion.path,
                format!("electrical.chargers.chargere.output.{out}.voltage")
            );
        }
    }

    #[test]
    fn a_trailing_space_in_the_article_still_matches() {
        // One shipping charger reports "44010250 ". The library trims on read,
        // but a hand-written mapping file might not.
        assert!(lookup("44010250 ", "0.5", 0x002, "x").is_some());
        assert!(knows_article(" 40021006"));
    }

    #[test]
    fn an_unknown_model_is_not_an_error() {
        assert!(lookup("99999999", "1.0", 0x001, "x").is_none());
        assert!(!knows_article("99999999"));
    }

    #[test]
    fn entries_carry_their_provenance() {
        let k = lookup("40021006", "7.9", 0x00E, "x").unwrap();
        assert!(k.model.contains("Mass"), "{}", k.model);
        assert!(k.source.contains("#6"), "{}", k.source);
        // No per-firmware overrides bundled yet, so this is the general list.
        assert!(!k.exact_firmware);
    }

    /// Every bundled path must be reachable from the unit its field reports on
    /// the bus this came from, or the sidecar would skip it.
    #[test]
    fn every_bundled_path_converts_from_its_real_unit() {
        // (article, field, unit as the device reports it) from the #6 dump.
        let cases: &[(&str, FieldId, &str)] = &[
            ("44010250", 0x009, "A"),
            ("44010250", 0x00C, "A"),
            ("44010250", 0x01D, "\u{b0}C"),
            ("44010250", 0x002, "V"),
            ("44010250", 0x004, "V"),
            ("44010250", 0x006, "V"),
            ("44010250", 0x000, ""),
            ("44010250", 0x001, ""),
            ("44010250", 0x01A, ""),
            ("40021006", 0x00E, "V"),
            ("40021006", 0x00F, "A"),
            ("40021006", 0x010, "\u{b0}C"),
            ("40021006", 0x011, "\u{b0}C"),
            ("40021006", 0x00D, ""),
            ("40021006", 0x012, ""),
            ("40021006", 0x015, ""),
            ("40021006", 0x018, ""),
        ];
        for (article, field, unit) in cases {
            let k = lookup(article, "", *field, "x")
                .unwrap_or_else(|| panic!("{article}/{field:#05X} should be bundled"));
            assert!(
                conversion(unit, leaf_unit(&k.suggestion.path)).is_some(),
                "{article}/{field:#05X} → {} cannot be reached from {unit:?}",
                k.suggestion.path
            );
        }
    }

    /// Nothing in the catalog should still contain the placeholder after a
    /// lookup, and nothing should be missing it before one.
    #[test]
    fn every_path_is_instance_templated() {
        for article in articles() {
            let m = &index().models[article];
            for (id, e) in m.any.iter() {
                assert!(
                    e.path.contains("{instance}"),
                    "{article}/{id:#05X} has no {{instance}} placeholder: {}",
                    e.path
                );
            }
        }
        let k = lookup("40021006", "", 0x00E, "house").unwrap();
        assert!(!k.suggestion.path.contains("{instance}"));
        assert!(k.suggestion.path.contains(".house."));
    }
}
