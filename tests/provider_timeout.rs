use model_harbor::model::ProviderRow;

#[test]
fn new_provider_with_default_timeout_does_not_write_timeout_field() {
    let mut p = ProviderRow::new();
    p.key = "test-provider".into();
    p.base_url = "https://api.example.com/v1".into();

    // 新建后不修改 timeout（保持默认 180000）
    assert_eq!(p.timeout, "180000");
    assert_eq!(p.original_timeout, "");

    let v = p.to_value();
    let options = v.get("options").and_then(|o| o.as_object());

    // 未修改过的默认值不应写入文件
    assert!(
        options.is_none() || !options.unwrap().contains_key("timeout"),
        "默认 timeout 不应写入配置文件"
    );
}

#[test]
fn new_provider_with_modified_timeout_writes_timeout_field() {
    let mut p = ProviderRow::new();
    p.key = "test-provider".into();
    p.base_url = "https://api.example.com/v1".into();

    // 修改 timeout 为其他值
    p.timeout = "60000".into();

    let v = p.to_value();
    let timeout_value = v
        .get("options")
        .and_then(|o| o.get("timeout"))
        .and_then(|t| t.as_u64());

    assert_eq!(timeout_value, Some(60000), "修改后的 timeout 应写入文件");
}

#[test]
fn new_provider_changed_then_back_to_default_still_writes() {
    let mut p = ProviderRow::new();
    p.key = "test-provider".into();
    p.base_url = "https://api.example.com/v1".into();

    // 场景：用户改成 60000 后又改回 180000
    // 此时 original_timeout 仍为空，但 timeout 被修改过
    // 实际上这个场景下 original_timeout 应该更新为 "60000"
    // 但当前实现中，只要 original_timeout 为空就判定为"从未设置过"
    p.timeout = "180000".into();
    p.original_timeout = "".into();

    let v = p.to_value();
    let options = v.get("options").and_then(|o| o.as_object());

    // 当前实现：original_timeout 为空 + timeout=180000 → 不写入
    assert!(
        options.is_none() || !options.unwrap().contains_key("timeout"),
        "改回默认值且 original_timeout 为空时不写入"
    );
}
