use super::*;

/// One of the model field edit rows: id / name / reasoning / tool_call / store / context / output.
///
/// `relaxed` = add-form context: content toggles relax to always-visible in `ProviderFormFlags`,
/// while dialect gates (oc / dsh) stay.
///
/// `dup` supplies the model-id duplicate set (the edit form): a hit draws a ⚠ beside the id field;
/// the add form passes `None`.
///
/// `salt` distinguishes a page's multiple form instances: dropdowns record expand state in egui by
/// `Id`, and two places sharing one id would tangle each other's open/closed state.
///
/// `sync` collects this frame's clicked "sync" targets (context / output): a hit writes and the caller
/// bubbles it up to the block level for centralized application. The add form (`relaxed`) offers no sync.
pub(super) fn model_core_fields_row(
    ui: &mut egui::Ui,
    m: &mut ModelRow,
    flags: &ProviderFormFlags,
    relaxed: bool,
    dup: Option<(&HashSet<String>, &HashSet<String>)>,
    salt: &str,
    sync: &mut Option<SyncField>,
) {
    let on = |flag: bool| relaxed || flag;
    ui.horizontal_wrapped(|ui| {
        field_label(ui, 120.0, "id:");
        let id_resp = ui.add(egui::TextEdit::singleline(&mut m.id).desired_width(120.0));
        if let Some((same_provider, global)) = dup {
            if !m.id.trim().is_empty()
                && (same_provider.contains(m.id.trim()) || global.contains(m.id.trim()))
            {
                // Saving doesn't fail on duplicate model ids — only duplicate provider keys block.
                // WorkBuddy is the only backend deduping globally by id: of duplicates, only the first takes effect.
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
        // The v2 schema has no reasoning key: hidden even in the add form (relaxed);
        // crossing to a v1 target lands reasoning: true via the default-checked value.
        if on(flags.show_model_reasoning) && !flags.oc_v2 && (flags.show_oc || !flags.show_dsh) {
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
            // The add form offers no sync (this model isn't in the table yet — no "all models" to brush).
            let sync_tip = if relaxed {
                None
            } else {
                Some("把当前模型的上下文值同步到本页全部模型")
            };
            if preset_combo(
                ui,
                &mut m.context,
                &CONTEXT_PRESETS,
                &format!("ctx_preset_{}", salt),
                "常用上下文预设值；选择即填入，之后仍可手动改。",
                sync_tip,
            ) {
                *sync = Some(SyncField::Context);
            }
        }
        if on(flags.show_model_output) {
            field_label(ui, 120.0, flags.output_label);
            numeric_text_edit(ui, &mut m.output, 53.0, "");
            let sync_tip = if relaxed {
                None
            } else {
                Some("把当前模型的最大输出值同步到本页全部模型")
            };
            if preset_combo(
                ui,
                &mut m.output,
                &OUTPUT_PRESETS,
                &format!("out_preset_{}", salt),
                "常用最大输出预设值；选择即填入，之后仍可手动改。",
                sync_tip,
            ) {
                *sync = Some(SyncField::Output);
            }
        }
    });
}

/// Model field edit row two: modalities (input / output) and the tiers (variants) dropdown.
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
            field_label(ui, 120.0, flags.modal_output_label);
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

/// The "add model" sub-form (shared by both forms): field rows + the add button.
///
/// On add, the model enters `p.models` and the sub-form clears and collapses (`show_key` keys the expand state).
pub(super) fn new_model_subform(
    ui: &mut egui::Ui,
    p: &mut ProviderRow,
    flags: &ProviderFormFlags,
    variants: &mut VariantsCtx<'_>,
    show_key: &str,
) {
    // `show_key` is unique per form (the edit form carries the provider key), serving as the preset dropdown's id salt.
    // The add form offers no sync, so the passed-in sync out-param is always None.
    // The draft schema matches its provider: new entries in a v2 file never land in the v1 shape.
    p.new_model.schema = p.schema;
    model_core_fields_row(ui, &mut p.new_model, flags, true, None, show_key, &mut None);
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
