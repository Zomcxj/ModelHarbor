use super::{preset_index, preset_label, preset_value, CONTEXT_PRESETS, OUTPUT_PRESETS};

/// 写入配置的必须是**纯数字**：上游不认 `128k` 这种写法，而 `numeric_text_edit`
/// 也会把它标红（非法数字 → 保存时字段被忽略，等于白填）。
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

/// 标签与写入值必须成对：`128k` ↔ `128000`。两者分开算，写错了界面会显示一个
/// 数字、填进去另一个，而用户只会看到自己选的「128k」。
#[test]
fn the_label_and_the_written_value_agree() {
    assert_eq!(preset_label(128), "128k");
    assert_eq!(preset_value(128), "128000");
    assert_eq!(preset_label(1024), "1024k");
    // 1024k 取 1000 进制（1024000）而不是 1048576：这些字段是发给上游的声明，
    // 少声明是安全的、多声明会被直接拒绝。
    assert_eq!(preset_value(1024), "1024000");
    // 131 / 262 对应厂商的 131072 / 262144，同样按 1000 进制落数。
    assert_eq!(preset_label(131), "131k");
    assert_eq!(preset_value(131), "131000");
    assert_eq!(preset_value(262), "262000");
}

/// 预设表要覆盖用户点名的那几个值，且从小到大排好（下拉里的顺序就是它）。
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

/// 仓库里的占位值 `262000` 应当正好命中一个预设，否则新模型的下拉会显示
/// 「选择...」而看不出自己其实已经是 262k。
#[test]
fn the_built_in_placeholder_matches_a_preset() {
    let placeholder = crate::model::ModelRow::new().context;
    assert_eq!(placeholder, preset_value(262));
    let output = crate::model::ModelRow::new().output;
    assert_eq!(output, preset_value(131));
}

/// 手填的非预设值不能被误认成某个预设——否则下拉会显示一个用户没选过的标签，
/// 让人以为自己选过。
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

/// 反查要能对上：每个预设值本身，以及两侧带空白的写法，都要认出来。
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
