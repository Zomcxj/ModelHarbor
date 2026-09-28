use super::*;

/// 模型字段编辑行之一：id / name / reasoning / tool_call / store / context / output。
///
/// `relaxed` = 新增表单语境：`ProviderFormFlags` 里**内容型**开关（文件里出现过该键
/// 才显示）放宽为常显——新模型是全新条目，不该受已加载文件内容影响；
/// 方言门控（oc / dsh）照旧保留。
///
/// `dup` 给出模型 id 的重复判定集合（编辑表单）：命中时在 id 字段旁画 ⚠ 提示——
/// 只有编辑表单有这步，新增表单传 None。
///
/// `salt` 区分同一页里的多个表单实例（编辑表单用 provider key，新增表单用固定串）：
/// 下拉的展开状态按 `Id` 记在 egui 里，两处共用同一个 id 会互相串开合状态。
pub(super) fn model_core_fields_row(
    ui: &mut egui::Ui,
    m: &mut ModelRow,
    flags: &ProviderFormFlags,
    relaxed: bool,
    dup: Option<(&HashSet<String>, &HashSet<String>)>,
    salt: &str,
) {
    let on = |flag: bool| relaxed || flag;
    ui.horizontal_wrapped(|ui| {
        field_label(ui, 120.0, "id:");
        let id_resp = ui.add(egui::TextEdit::singleline(&mut m.id).desired_width(120.0));
        if let Some((same_provider, global)) = dup {
            if !m.id.trim().is_empty()
                && (same_provider.contains(m.id.trim()) || global.contains(m.id.trim()))
            {
                // 保存**不会**因为模型 id 重复而失败（只有 provider key 重复才拦），
                // 所以这里不能说「保存将被阻止」。WorkBuddy 是唯一按 id 全局去重的
                // 后端：重复的 id 里只有第一条会在它的选择器里生效。
                let hint = if flags.show_model_disabled {
                    "这个模型名在**全局**出现多次（含其他厂商）。\n\
                     WorkBuddy 的选择器按模型 id 全局去重，同名只会列出一行、\
                     只有第一条生效。\n\
                     要用哪一家，就在那一家的卡片上勾「启用」——勾上后同名的\
                     其他条目会自动关闭，只有勾选的写进 models.json。\n\
                     取消勾选不会丢配置：条目仍存在 models.full.json 里，勾回来即可。\n\
                     不要改 id —— id 就是发给上游的模型名，改了会直接请求失败。"
                } else {
                    "id 与同 provider 内其他模型重复；保存仍会写入，\
                     但同名模型在部分后端只会生效一次。"
                };
                id_resp.on_hover_text(hint);
                ui.label(
                    egui::RichText::new("⚠ 重复")
                        .small()
                        .color(crate::theme::semantics(ui).warn),
                );
            }
        }
        if on(flags.show_model_name) {
            field_label(ui, 120.0, "name:");
            ui.add(egui::TextEdit::singleline(&mut m.name).desired_width(120.0));
        }
        if on(flags.show_model_reasoning) && (flags.show_oc || !flags.show_dsh) {
            ui.checkbox(&mut m.reasoning, "reasoning");
        }
        if on(flags.show_model_tool_call) && flags.show_oc {
            ui.checkbox(&mut m.tool_call, "tool_call");
        }
        if on(flags.show_model_store) && flags.show_oc {
            ui.checkbox(&mut m.store, "store");
        }
        if on(flags.show_model_context) {
            field_label(ui, 120.0, flags.context_label);
            numeric_text_edit(ui, &mut m.context, 53.0, "");
            preset_combo(
                ui,
                &mut m.context,
                &CONTEXT_PRESETS,
                &format!("ctx_preset_{}", salt),
                "常用上下文预设值；选择即填入，之后仍可手动改。",
            );
        }
        if on(flags.show_model_output) {
            field_label(ui, 120.0, flags.output_label);
            numeric_text_edit(ui, &mut m.output, 53.0, "");
            preset_combo(
                ui,
                &mut m.output,
                &OUTPUT_PRESETS,
                &format!("out_preset_{}", salt),
                "常用最大输出预设值；选择即填入，之后仍可手动改。",
            );
        }
    });
}

/// 模型字段编辑行之二：模态（input / output）与档位（variants）下拉。
pub(super) fn model_modalities_row(
    ui: &mut egui::Ui,
    m: &mut ModelRow,
    flags: &ProviderFormFlags,
    variants: &mut VariantsCtx<'_>,
    relaxed: bool,
) {
    let on = |flag: bool| relaxed || flag;
    ui.horizontal_wrapped(|ui| {
        if on(flags.show_model_input) {
            field_label(ui, 120.0, flags.input_label);
            ui.add(egui::TextEdit::singleline(&mut m.modalities_input).desired_width(80.0));
        }
        if flags.show_oc {
            field_label(ui, 120.0, "modalities.output");
            ui.add(egui::TextEdit::singleline(&mut m.modalities_output).desired_width(80.0));
        }
        if on(flags.show_model_variants) {
            field_label(ui, 120.0, variants.label);
        }
        variant_selector(
            ui,
            &mut m.variants,
            variants.names,
            variants.open_key.clone(),
            variants.open_set,
            variants.normalize,
        );
    });
}

/// 「添加 Model」子表单（两份表单共用）：字段行 + 添加按钮。
///
/// 添加后模型进入 `p.models`、子表单清空并收起（`show_key` 是展开状态的键）。
pub(super) fn new_model_subform(
    ui: &mut egui::Ui,
    p: &mut ProviderRow,
    flags: &ProviderFormFlags,
    variants: &mut VariantsCtx<'_>,
    show_key: &str,
) {
    // `show_key` 按构造就逐表单唯一（编辑表单带 provider key），正好当预设下拉的
    // id 盐用——不必再单传一个参数。
    model_core_fields_row(ui, &mut p.new_model, flags, true, None, show_key);
    model_modalities_row(ui, &mut p.new_model, flags, variants, true);
    ui.horizontal(|ui| {
        ui.add_space(crate::theme::SPACE_7);
        if ui.button("添加").clicked() && !p.new_model.id.trim().is_empty() {
            p.models.push(p.new_model.clone());
            p.new_model = ModelRow::new();
            variants.open_set.remove(show_key);
        }
    });
}
