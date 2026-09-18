use model_harbor::model::ProviderRow;

#[test]
fn new_provider_default_timeout_is_saved() {
    let p = ProviderRow::new();
    assert_eq!(p.timeout, "180000");
    assert_eq!(p.original_timeout, "");

    let v = p.to_value();
    let timeout_in_json = v
        .get("options")
        .and_then(|o| o.get("timeout"))
        .and_then(|t| t.as_u64());

    // 新建 provider 默认 timeout 180000 也应写入配置文件
    assert_eq!(timeout_in_json, Some(180000));
}

#[test]
fn new_provider_modified_timeout_is_saved() {
    let mut p = ProviderRow::new();
    p.timeout = "60000".to_string();

    let v = p.to_value();
    let timeout_in_json = v
        .get("options")
        .and_then(|o| o.get("timeout"))
        .and_then(|t| t.as_u64());

    // 修改过的 timeout 应该写入
    assert_eq!(timeout_in_json, Some(60000));
}
