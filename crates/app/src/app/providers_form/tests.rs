use super::{preset_index, preset_label, preset_value, CONTEXT_PRESETS, OUTPUT_PRESETS};

/// The value written into the config must be a pure number.
#[test]
fn every_preset_writes_a_plain_integer() {
    for k in CONTEXT_PRESETS.iter().chain(OUTPUT_PRESETS.iter()) {
        let value = preset_value(*k);
        assert!(
            value.chars().all(|c| c.is_ascii_digit()),
            "{} 的写入值不是纯数字：{}",
            k,
            value
        );
        assert!(
            crate::util::parse_number_text(&value).is_some(),
            "{} 的写入值过不了表单的数字校验：{}",
            k,
            value
        );
    }
}

/// Labels pair with written values: `128k` ↔ `128000`.
#[test]
fn the_label_and_the_written_value_agree() {
    assert_eq!(preset_label(128), "128k");
    assert_eq!(preset_value(128), "128000");
    assert_eq!(preset_label(1024), "1024k");
    // 1024k takes the decimal reading (1024000), not 1048576.
    assert_eq!(preset_value(1024), "1024000");
    // 131 / 262 match vendors' 131072 / 262144, landing decimal the same way.
    assert_eq!(preset_label(131), "131k");
    assert_eq!(preset_value(131), "131000");
    assert_eq!(preset_value(262), "262000");
}

/// The preset table covers the given values, ascending (the dropdown's order).
#[test]
fn the_tables_hold_the_requested_values_in_order() {
    assert_eq!(CONTEXT_PRESETS, [128, 200, 262, 300, 400, 500, 1024]);
    assert_eq!(OUTPUT_PRESETS, [32, 64, 131, 262]);
    for table in [&CONTEXT_PRESETS[..], &OUTPUT_PRESETS[..]] {
        assert!(
            table.windows(2).all(|w| w[0] < w[1]),
            "预设必须严格递增：{:?}",
            table
        );
    }
}

/// The placeholder `262000` hits a preset, so a new model's dropdown can show its current tier.
#[test]
fn the_built_in_placeholder_matches_a_preset() {
    let placeholder = crate::model::ModelRow::new().context;
    assert_eq!(placeholder, preset_value(262));
    let output = crate::model::ModelRow::new().output;
    assert_eq!(output, preset_value(131));
}

/// Hand-typed non-preset values are never mistaken for a preset.
#[test]
fn a_hand_typed_value_is_not_mistaken_for_a_preset() {
    for value in ["", "  ", "1000", "100000", "999999", "abc", "128000000"] {
        assert_eq!(
            preset_index(value, &CONTEXT_PRESETS),
            None,
            "{} 不该命中任何预设",
            value
        );
    }
}

/// Reverse lookup: every preset value itself, plus whitespace-padded spellings on both sides, must be recognized.
#[test]
fn the_current_value_is_recognised_after_a_round_trip() {
    for (i, k) in CONTEXT_PRESETS.iter().enumerate() {
        let value = preset_value(*k);
        assert_eq!(preset_index(&value, &CONTEXT_PRESETS), Some(i));
        assert_eq!(
            preset_index(&format!("  {}  ", value), &CONTEXT_PRESETS),
            Some(i),
            "两侧空白不该影响识别"
        );
    }
}
