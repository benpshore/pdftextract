//! `AcroForm` and static XFA operations on immutable inputs.

use std::collections::{BTreeMap, HashSet};

use pdf_oxide::editor::{DocumentEditor, FormFieldValue, FormFieldWrapper};
use pdf_oxide::extractors::forms::{FieldType, field_flags};
use pdf_oxide::fdf::{FdfField, FdfValue, FdfWriter, XfdfWriter};
use pdf_oxide::object::Object;
use pdf_oxide::writer::{
    CheckboxWidget, ComboBoxWidget, FormFieldWidget, ListBoxWidget, PdfWriter, PushButtonWidget,
    SignatureWidget, TextFieldWidget,
};
use pdf_oxide::xfa::{XfaConverter, XfaExtractor, XfaParser, add_converted_page};
use quick_xml::Reader;
use quick_xml::events::Event;
use serde::Deserialize;
use serde_json::{Value, json};

use super::edit::{
    bounded_count, bounded_string, edit_document, page_index, parse, pdf_artifact, pdf_string,
    rectangle, repair_error, repair_object, repaired_output, rgb, saved, upstream,
};
use super::{Artifact, Context, OperationOutput};
use crate::backend::BackendError;

pub const OPERATIONS: &[&str] = &[
    "forms.inspect",
    "forms.add",
    "forms.fill",
    "forms.update",
    "forms.remove",
    "forms.flatten",
    "forms.export",
    "forms.xfa",
];

pub(super) fn execute(
    operation: &str,
    context: &Context<'_>,
    params: &Value,
) -> Option<Result<OperationOutput, BackendError>> {
    Some(match operation {
        "forms.inspect" => inspect(context, params),
        "forms.add" => add(context, params),
        "forms.fill" => fill(context, params),
        "forms.update" => update(context, params),
        "forms.remove" => remove(context, params),
        "forms.flatten" => flatten(context, params),
        "forms.export" => export(context, params),
        "forms.xfa" => xfa(context, params),
        _ => return None,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

fn fields(
    context: &Context<'_>,
    editor: &mut DocumentEditor,
) -> Result<BTreeMap<String, FormFieldWrapper>, BackendError> {
    super::security::validate_form_tree(editor.source(), context)?;
    let fields = editor.get_form_fields().map_err(upstream)?;
    bounded_count(fields.len(), context.limits.max_items, "form fields")?;
    let mut result = BTreeMap::new();
    for field in fields {
        bounded_string(field.name(), "field name")?;
        if result.insert(field.name().to_owned(), field).is_some() {
            return Err(BackendError::Unsupported(
                "ambiguous duplicate fully qualified form field names".into(),
            ));
        }
    }
    Ok(result)
}

fn value_json(value: &FormFieldValue) -> Value {
    match value {
        FormFieldValue::Text(value) => json!({"kind": "text", "value": value}),
        FormFieldValue::Boolean(value) => json!({"kind": "boolean", "value": value}),
        FormFieldValue::Choice(value) => json!({"kind": "choice", "value": value}),
        FormFieldValue::MultiChoice(value) => json!({"kind": "multi_choice", "value": value}),
        FormFieldValue::None => json!({"kind": "none"}),
    }
}

fn field_rectangle(field: &FormFieldWrapper) -> Option<[f32; 4]> {
    // FormExtractor correctly stores PDF corner coordinates. The pinned
    // FormFieldWrapper::bounds incorrectly treats x1/y1 as width/height.
    if let Some(corners) = field.original().and_then(|field| field.bounds) {
        return Some([
            corners[0] as f32,
            corners[1] as f32,
            (corners[2] - corners[0]) as f32,
            (corners[3] - corners[1]) as f32,
        ]);
    }
    field.widget_config().map(|config| {
        [
            config.rect.x,
            config.rect.y,
            config.rect.width,
            config.rect.height,
        ]
    })
}

fn resolve_form_object(
    document: &pdf_oxide::PdfDocument,
    object: &Object,
) -> Result<Object, BackendError> {
    let mut current = object.clone();
    let mut seen = HashSet::new();
    for _ in 0..8 {
        let Object::Reference(reference) = current else {
            return Ok(current);
        };
        if !seen.insert(reference) {
            return Err(BackendError::Malformed(
                "cyclic form object reference".into(),
            ));
        }
        current = document.load_object(reference).map_err(upstream)?;
    }
    if matches!(current, Object::Reference(_)) {
        return Err(BackendError::Limit(
            "form object reference depth exceeds 8".into(),
        ));
    }
    Ok(current)
}

fn tree_has_appearance(
    context: &Context<'_>,
    document: &pdf_oxide::PdfDocument,
    root: Object,
) -> Result<bool, BackendError> {
    // Inspect only Fields/Kids edges, never Parent or appearance resources. A
    // field can own separate anonymous widget children, so checking its own AP
    // alone would incorrectly admit fills that preserve a child's stale stream.
    let mut pending = vec![(root, 0_usize)];
    let mut seen = HashSet::new();
    let mut visited = 0_usize;
    while let Some((object, depth)) = pending.pop() {
        visited = visited.saturating_add(1);
        bounded_count(visited, context.limits.max_items, "form appearance nodes")?;
        if depth > 64 {
            return Err(BackendError::Limit(
                "form appearance tree depth exceeds 64".into(),
            ));
        }
        if let Object::Reference(reference) = &object
            && !seen.insert(*reference)
        {
            return Err(BackendError::Malformed(
                "cyclic or repeated form appearance node".into(),
            ));
        }
        let object = resolve_form_object(document, &object)?;
        match object {
            Object::Array(children) => {
                bounded_count(
                    visited
                        .saturating_add(pending.len())
                        .saturating_add(children.len()),
                    context.limits.max_items,
                    "form appearance nodes",
                )?;
                pending.extend(children.into_iter().map(|child| (child, depth + 1)));
            }
            Object::Dictionary(mut dict) => {
                // Even a malformed, empty or indirect AP is conservatively
                // appearance-bearing; this guard must not bless stale output.
                if dict
                    .get("AP")
                    .is_some_and(|value| !matches!(value, Object::Null))
                {
                    return Ok(true);
                }
                if let Some(children) = dict.remove("Kids") {
                    pending.push((children, depth + 1));
                }
            }
            _ => {
                return Err(BackendError::Malformed(
                    "form appearance tree must contain field dictionaries or arrays".into(),
                ));
            }
        }
    }
    Ok(false)
}

fn reject_stale_field_appearance(
    context: &Context<'_>,
    editor: &DocumentEditor,
    field: &FormFieldWrapper,
) -> Result<(), BackendError> {
    let reference = field
        .original()
        .and_then(|field| field.object_ref)
        .ok_or_else(|| {
            BackendError::Unsupported(
                "form appearance admission requires an unambiguous indirect field identity".into(),
            )
        })?;
    if tree_has_appearance(context, editor.source(), Object::Reference(reference))? {
        return Err(BackendError::Unsupported(format!(
            "field {} has an existing /AP appearance; pinned value/style updates do not regenerate it and can leave visible stale content",
            field.name()
        )));
    }
    Ok(())
}

fn reject_pending_appearance_flatten(
    context: &Context<'_>,
    editor: &DocumentEditor,
) -> Result<(), BackendError> {
    let document = editor.source();
    let catalog = document.catalog().map_err(upstream)?;
    let form = catalog.as_dict().and_then(|dict| dict.get("AcroForm"));
    let Some(form) = form else {
        return Ok(());
    };
    let form = resolve_form_object(document, form)?;
    let dict = form
        .as_dict()
        .ok_or_else(|| BackendError::Malformed("AcroForm must resolve to a dictionary".into()))?;
    if matches!(dict.get("NeedAppearances"), Some(Object::Boolean(true)))
        && let Some(roots) = dict.get("Fields")
        && tree_has_appearance(context, document, roots.clone())?
    {
        return Err(BackendError::Unsupported(
            "AcroForm requests /NeedAppearances and retains /AP streams; pinned flattening can bake stale values from an earlier editing session".into(),
        ));
    }
    Ok(())
}

fn inspect(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    let _: Empty = parse(params)?;
    document_scope(context)?;
    // Inspection can authenticate encrypted inputs without creating a plaintext PDF.
    let mut editor = DocumentEditor::from_document(context.open_pdf()?).map_err(upstream)?;
    bounded_count(
        editor.current_page_count(),
        context.limits.max_pages as usize,
        "form source pages",
    )?;
    let fields = fields(context, &mut editor)?;
    let records = fields
        .values()
        .map(|field| {
            let rect = field_rectangle(field);
            json!({"name": field.name(), "type": field.field_type().map(|t| format!("{t:?}")),
            "value": value_json(&field.value()), "rect": rect,
            "tooltip": field.tooltip(), "flags": field.flags(),
            "readonly": field.is_readonly(), "required": field.is_required(),
            "no_export": field.is_no_export(), "max_length": field.get_max_length(),
            "alignment": field.get_alignment(), "parent": field.parent_name(),
            "is_parent": field.is_parent_only()})
        })
        .collect::<Vec<_>>();
    Ok(OperationOutput {
        data: json!({"fields": records, "has_xfa": editor.has_xfa().map_err(upstream)?}),
        warnings: vec!["Field widgets may span pages; a defaulted or first-widget page number is not reported as exact provenance.".into()],
        ..Default::default()
    })
}

fn document_scope(context: &Context<'_>) -> Result<(), BackendError> {
    if context.pages.is_some() {
        return Err(BackendError::Unsupported(
            "this form operation is document-scoped; page-restricted form changes require unambiguous widget provenance".into(),
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum ValueParam {
    Text(String),
    Boolean(bool),
    Choice(String),
    MultiChoice(Vec<String>),
    None,
}

impl ValueParam {
    fn convert(self, context: &Context<'_>) -> Result<FormFieldValue, BackendError> {
        Ok(match self {
            Self::Text(value) => {
                bounded_string(&value, "field text")?;
                FormFieldValue::Text(value)
            }
            Self::Boolean(value) => FormFieldValue::Boolean(value),
            Self::Choice(value) => {
                bounded_string(&value, "field choice")?;
                FormFieldValue::Choice(value)
            }
            Self::MultiChoice(values) => {
                bounded_count(values.len(), context.limits.max_items, "field selections")?;
                for value in &values {
                    bounded_string(value, "field choice")?;
                }
                FormFieldValue::MultiChoice(values)
            }
            Self::None => FormFieldValue::None,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FillParams {
    fields: BTreeMap<String, ValueParam>,
}

fn verify_fields(
    context: &Context<'_>,
    output: &OperationOutput,
) -> Result<BTreeMap<String, FormFieldWrapper>, BackendError> {
    let mut check =
        DocumentEditor::from_bytes(output.artifacts[0].bytes.clone()).map_err(upstream)?;
    fields(context, &mut check)
}

fn fill(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    document_scope(context)?;
    let params: FillParams = parse(params)?;
    bounded_count(
        params.fields.len(),
        context.limits.max_items,
        "field updates",
    )?;
    let mut editor = edit_document(context)?;
    let original = fields(context, &mut editor)?;
    let mut expected = BTreeMap::new();
    for (name, value) in params.fields {
        let field = original
            .get(&name)
            .ok_or_else(|| BackendError::Malformed(format!("form field does not exist: {name}")))?;
        if field.is_readonly() {
            return Err(BackendError::Unsupported(format!(
                "form field is read-only: {name}"
            )));
        }
        let value = value.convert(context)?;
        let valid = match field.field_type() {
            Some(FieldType::Text) => {
                matches!(value, FormFieldValue::Text(_) | FormFieldValue::None)
            }
            Some(FieldType::Button) => {
                matches!(
                    value,
                    FormFieldValue::Boolean(_) | FormFieldValue::Choice(_) | FormFieldValue::None
                ) && field.flags().unwrap_or(0) & field_flags::PUSH_BUTTON == 0
            }
            Some(FieldType::Choice) => matches!(
                value,
                FormFieldValue::Choice(_) | FormFieldValue::MultiChoice(_) | FormFieldValue::None
            ),
            _ => false,
        };
        if !valid {
            return Err(BackendError::Unsupported(format!(
                "value type is incompatible with field {name}; signature values are never written"
            )));
        }
        if let FormFieldValue::Text(text) = &value
            && field
                .get_max_length()
                .is_some_and(|max| text.chars().count() > max as usize)
        {
            return Err(BackendError::Limit(format!(
                "field {name} exceeds its MaxLen"
            )));
        }
        if let FormFieldValue::MultiChoice(values) = &value
            && values.len() > 1
            && field.flags().unwrap_or(0) & field_flags::MULTI_SELECT == 0
        {
            return Err(BackendError::Malformed(format!(
                "field {name} is not multi-select"
            )));
        }
        if value != field.value() {
            reject_stale_field_appearance(context, &editor, field)?;
            editor
                .set_form_field_value(&name, value.clone())
                .map_err(upstream)?;
        }
        expected.insert(name, value);
    }
    let output = saved(
        context,
        &mut editor,
        json!({"updated_fields": expected.keys().collect::<Vec<_>>() }),
    )?;
    let actual = verify_fields(context, &output)?;
    for (name, value) in expected {
        if actual.get(&name).is_none_or(|field| field.value() != value) {
            return Err(BackendError::Unsupported(format!(
                "pinned writer failed exact value round-trip for field {name}"
            )));
        }
    }
    Ok(output)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddParams {
    fields: Vec<NewField>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewField {
    name: String,
    page: u32,
    rect: [f32; 4],
    widget: Widget,
    #[serde(default)]
    readonly: bool,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    tooltip: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Widget {
    Text {
        #[serde(default)]
        value: String,
        #[serde(default)]
        multiline: bool,
        #[serde(default)]
        max_length: Option<u32>,
    },
    Checkbox {
        #[serde(default)]
        checked: bool,
    },
    Combo {
        options: Vec<String>,
        #[serde(default)]
        value: Option<String>,
        #[serde(default)]
        editable: bool,
    },
    List {
        options: Vec<String>,
        #[serde(default)]
        values: Vec<String>,
        #[serde(default)]
        multi_select: bool,
    },
    PushButton {
        caption: String,
    },
    /// Empty signature widget only; this does not sign a document.
    Signature,
}

fn add_widget<W: FormFieldWidget>(
    editor: &mut DocumentEditor,
    page: usize,
    widget: W,
    expected: &str,
) -> Result<(), BackendError> {
    let actual = if expected.contains('.') {
        editor
            .add_form_field_hierarchical(page, widget)
            .map_err(upstream)?
    } else {
        editor.add_form_field(page, widget).map_err(upstream)?
    };
    if actual != expected {
        return Err(BackendError::Malformed(
            "upstream renamed the requested form field".into(),
        ));
    }
    Ok(())
}

fn options(values: &[String], context: &Context<'_>) -> Result<(), BackendError> {
    bounded_count(values.len(), context.limits.max_items, "choice options")?;
    if values.is_empty() {
        return Err(BackendError::Malformed(
            "choice options cannot be empty".into(),
        ));
    }
    let mut seen = HashSet::new();
    for value in values {
        bounded_string(value, "choice option")?;
        if !seen.insert(value) {
            return Err(BackendError::Malformed("duplicate choice option".into()));
        }
    }
    Ok(())
}

fn add(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    document_scope(context)?;
    let params: AddParams = parse(params)?;
    bounded_count(params.fields.len(), context.limits.max_items, "new fields")?;
    let mut editor = edit_document(context)?;
    let catalog = editor.source().catalog().map_err(upstream)?;
    if catalog
        .as_dict()
        .is_some_and(|dict| dict.contains_key("AcroForm"))
    {
        return Err(BackendError::Unsupported(
            "adding fields to an existing AcroForm is unsupported: the pinned writer rebuilds its catalog and can discard existing fields, resources and unique properties; preservation repair is not verified".into(),
        ));
    }
    let original = fields(context, &mut editor)?;
    bounded_count(
        original.len().saturating_add(params.fields.len()),
        context.limits.max_items,
        "total fields",
    )?;
    let count = editor.current_page_count();
    let mut names = HashSet::new();
    let mut property_repairs = Vec::new();
    for field in params.fields {
        bounded_string(&field.name, "field name")?;
        if field.name.is_empty()
            || field.name.split('.').any(str::is_empty)
            || original.contains_key(&field.name)
            || !names.insert(field.name.clone())
        {
            return Err(BackendError::Malformed(
                "field names must be nonempty, unique qualified names".into(),
            ));
        }
        let page = page_index(field.page, count)?;
        let rect = rectangle(field.rect)?;
        property_repairs.push(Properties {
            name: field.name.clone(),
            readonly: Some(field.readonly),
            required: Some(field.required),
            tooltip: field.tooltip.clone(),
            rect: None,
            max_length: None,
            alignment: None,
            background_color: None,
            border_color: None,
            border_width: None,
        });
        match field.widget {
            Widget::Text {
                value,
                multiline,
                max_length,
            } => {
                bounded_string(&value, "field text")?;
                let mut widget = TextFieldWidget::new(&field.name, rect).with_value(value.clone());
                if multiline {
                    widget = widget.multiline();
                }
                if let Some(maximum) = max_length {
                    if maximum == 0 || value.chars().count() > maximum as usize {
                        return Err(BackendError::Malformed(
                            "text exceeds MaxLen or MaxLen is zero".into(),
                        ));
                    }
                    widget = widget.with_max_length(maximum);
                }
                add_widget(&mut editor, page, widget, &field.name)?;
            }
            Widget::Checkbox { checked } => add_widget(
                &mut editor,
                page,
                CheckboxWidget::new(&field.name, rect).with_checked(checked),
                &field.name,
            )?,
            Widget::Combo {
                options: choices,
                value,
                editable,
            } => {
                options(&choices, context)?;
                let mut widget =
                    ComboBoxWidget::new(&field.name, rect).with_options(choices.clone());
                if let Some(value) = value {
                    bounded_string(&value, "combo value")?;
                    if !editable && !choices.contains(&value) {
                        return Err(BackendError::Malformed(
                            "combo value is not among options".into(),
                        ));
                    }
                    widget = widget.with_value(value);
                }
                if editable {
                    widget = widget.editable();
                }
                add_widget(&mut editor, page, widget, &field.name)?;
            }
            Widget::List {
                options: choices,
                values,
                multi_select,
            } => {
                options(&choices, context)?;
                bounded_count(values.len(), context.limits.max_items, "selected values")?;
                if (!multi_select && values.len() > 1)
                    || values.iter().any(|v| !choices.contains(v))
                {
                    return Err(BackendError::Malformed(
                        "list selections are not permitted by the widget options".into(),
                    ));
                }
                let mut widget = ListBoxWidget::new(&field.name, rect)
                    .with_options(choices)
                    .with_values(values);
                if multi_select {
                    widget = widget.multi_select();
                }
                add_widget(&mut editor, page, widget, &field.name)?;
            }
            Widget::PushButton { caption } => {
                bounded_string(&caption, "button caption")?;
                add_widget(
                    &mut editor,
                    page,
                    PushButtonWidget::new(&field.name, rect).with_caption(caption),
                    &field.name,
                )?;
            }
            Widget::Signature => add_widget(
                &mut editor,
                page,
                SignatureWidget::new(&field.name, rect),
                &field.name,
            )?,
        }
        editor
            .set_form_field_readonly(&field.name, field.readonly)
            .map_err(upstream)?;
        editor
            .set_form_field_required(&field.name, field.required)
            .map_err(upstream)?;
        if let Some(tooltip) = field.tooltip {
            bounded_string(&tooltip, "field tooltip")?;
            editor
                .set_form_field_tooltip(&field.name, tooltip)
                .map_err(upstream)?;
        }
    }
    let mut ordered_names: Vec<_> = names.into_iter().collect();
    ordered_names.sort();
    let output = saved(context, &mut editor, json!({"added_fields": ordered_names}))?;
    let output = repair_form_properties(context, output, &property_repairs)?;
    let actual = verify_fields(context, &output)?;
    if ordered_names.iter().any(|name| !actual.contains_key(name)) {
        return Err(BackendError::Unsupported(
            "saved document did not retain every requested field name".into(),
        ));
    }
    Ok(output)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateParams {
    fields: Vec<Properties>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Properties {
    name: String,
    #[serde(default)]
    readonly: Option<bool>,
    #[serde(default)]
    required: Option<bool>,
    #[serde(default)]
    tooltip: Option<String>,
    #[serde(default)]
    rect: Option<[f32; 4]>,
    #[serde(default)]
    max_length: Option<u32>,
    #[serde(default)]
    alignment: Option<u32>,
    #[serde(default)]
    background_color: Option<[f32; 3]>,
    #[serde(default)]
    border_color: Option<[f32; 3]>,
    #[serde(default)]
    border_width: Option<f32>,
}

fn update(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    document_scope(context)?;
    let params: UpdateParams = parse(params)?;
    let property_repairs = params.fields.clone();
    bounded_count(
        params.fields.len(),
        context.limits.max_items,
        "field property updates",
    )?;
    let mut editor = edit_document(context)?;
    let original = fields(context, &mut editor)?;
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    for properties in params.fields {
        let name = properties.name;
        if !seen.insert(name.clone()) {
            return Err(BackendError::Malformed(
                "duplicate form property update name".into(),
            ));
        }
        let field = original
            .get(&name)
            .ok_or_else(|| BackendError::Malformed(format!("form field does not exist: {name}")))?;
        if matches!(field.field_type(), Some(FieldType::Signature)) {
            return Err(BackendError::Unsupported(
                "signature fields cannot be modified".into(),
            ));
        }
        if properties.rect.is_some()
            || properties.max_length.is_some()
            || properties.alignment.is_some()
            || properties.background_color.is_some()
            || properties.border_color.is_some()
            || properties.border_width.is_some()
        {
            reject_stale_field_appearance(context, &editor, field)?;
        }
        if let Some(value) = properties.readonly {
            editor
                .set_form_field_readonly(&name, value)
                .map_err(upstream)?;
        }
        if let Some(value) = properties.required {
            editor
                .set_form_field_required(&name, value)
                .map_err(upstream)?;
        }
        if let Some(value) = properties.tooltip {
            bounded_string(&value, "tooltip")?;
            editor
                .set_form_field_tooltip(&name, value)
                .map_err(upstream)?;
        }
        if let Some(value) = properties.rect {
            editor
                .set_form_field_rect(&name, rectangle(value)?)
                .map_err(upstream)?;
        }
        if let Some(value) = properties.max_length {
            if value == 0 || !matches!(field.field_type(), Some(FieldType::Text)) {
                return Err(BackendError::Malformed(
                    "positive MaxLen is supported only on text fields".into(),
                ));
            }
            editor
                .set_form_field_max_length(&name, value)
                .map_err(upstream)?;
        }
        if let Some(value) = properties.alignment {
            if value > 2 {
                return Err(BackendError::Malformed(
                    "alignment must be 0, 1, or 2".into(),
                ));
            }
            editor
                .set_form_field_alignment(&name, value)
                .map_err(upstream)?;
        }
        if let Some(value) = properties.background_color {
            editor
                .set_form_field_background_color(&name, rgb(value)?)
                .map_err(upstream)?;
        }
        if let Some(value) = properties.border_color {
            editor
                .set_form_field_border_color(&name, rgb(value)?)
                .map_err(upstream)?;
        }
        if let Some(value) = properties.border_width {
            if !value.is_finite() || !(0.0..=1000.0).contains(&value) {
                return Err(BackendError::Malformed(
                    "border width must be finite and within 0..=1000".into(),
                ));
            }
            editor
                .set_form_field_border_width(&name, value)
                .map_err(upstream)?;
        }
        names.push(name);
    }
    let output = saved(context, &mut editor, json!({"updated_fields": names}))?;
    repair_form_properties(context, output, &property_repairs)
}

fn repair_form_properties(
    context: &Context<'_>,
    output: OperationOutput,
    changes: &[Properties],
) -> Result<OperationOutput, BackendError> {
    let pdf =
        pdf_oxide::PdfDocument::from_bytes(output.artifacts[0].bytes.clone()).map_err(upstream)?;
    super::security::validate_form_tree(&pdf, context)?;
    let extracted =
        pdf_oxide::extractors::forms::FormExtractor::extract_fields(&pdf).map_err(upstream)?;
    bounded_count(
        extracted.len(),
        context.limits.max_items,
        "serialized form fields",
    )?;
    let mut by_name = BTreeMap::new();
    for field in extracted {
        let name = field.full_name.clone();
        if by_name.insert(name, field).is_some() {
            return Err(BackendError::Unsupported(
                "ambiguous serialized field name during property repair".into(),
            ));
        }
    }
    let mut document =
        lopdf::Document::load_mem(&output.artifacts[0].bytes).map_err(repair_error)?;
    for change in changes {
        let field = by_name.get(&change.name).ok_or_else(|| {
            BackendError::Malformed("requested field missing after serialization".into())
        })?;
        let reference = field.object_ref.ok_or_else(|| {
            BackendError::Unsupported(
                "direct form field property repair requires an indirect field identity".into(),
            )
        })?;
        let id = (reference.id, reference.r#gen);
        let mut dict = document
            .get_object(id)
            .map_err(repair_error)?
            .as_dict()
            .map_err(repair_error)?
            .clone();
        if (change.rect.is_some()
            || change.background_color.is_some()
            || change.border_color.is_some()
            || change.border_width.is_some())
            && (dict.get(b"Subtype").and_then(lopdf::Object::as_name).ok()
                != Some(b"Widget".as_slice())
                || dict.has(b"Kids"))
        {
            return Err(BackendError::Unsupported(
                "geometry and appearance updates require one merged field/widget identity".into(),
            ));
        }
        let mut flags = field.flags.unwrap_or(0);
        for (value, bit) in [(change.readonly, 1_u32), (change.required, 2_u32)] {
            if let Some(value) = value {
                if value {
                    flags |= bit;
                } else {
                    flags &= !bit;
                }
            }
        }
        if change.readonly.is_some() || change.required.is_some() {
            dict.set("Ff", i64::from(flags));
        }
        if let Some(tooltip) = &change.tooltip {
            dict.set("TU", pdf_string(tooltip));
        }
        if let Some(rect) = change.rect {
            let rect = rectangle(rect)?;
            dict.set(
                "Rect",
                vec![
                    rect.x.into(),
                    rect.y.into(),
                    (rect.x + rect.width).into(),
                    (rect.y + rect.height).into(),
                ],
            );
        }
        if let Some(max_length) = change.max_length {
            dict.set("MaxLen", i64::from(max_length));
        }
        if let Some(alignment) = change.alignment {
            dict.set("Q", i64::from(alignment));
        }
        if change.background_color.is_some() || change.border_color.is_some() {
            let mut appearance = match dict.get(b"MK") {
                Ok(object) => repair_object(&document, object)?
                    .1
                    .as_dict()
                    .map_err(repair_error)?
                    .clone(),
                Err(_) => lopdf::Dictionary::new(),
            };
            if let Some(color) = change.background_color {
                appearance.set(
                    "BG",
                    color
                        .into_iter()
                        .map(lopdf::Object::Real)
                        .collect::<Vec<_>>(),
                );
            }
            if let Some(color) = change.border_color {
                appearance.set(
                    "BC",
                    color
                        .into_iter()
                        .map(lopdf::Object::Real)
                        .collect::<Vec<_>>(),
                );
            }
            dict.set("MK", appearance);
        }
        if let Some(width) = change.border_width {
            let mut border = match dict.get(b"BS") {
                Ok(object) => repair_object(&document, object)?
                    .1
                    .as_dict()
                    .map_err(repair_error)?
                    .clone(),
                Err(_) => lopdf::Dictionary::new(),
            };
            border.set("W", width);
            dict.set("BS", border);
        }
        document.objects.insert(id, lopdf::Object::Dictionary(dict));
    }
    let output = repaired_output(
        context,
        output,
        document,
        "persist staged AcroForm properties omitted by the pinned editor writer",
    )?;
    let checked = verify_fields(context, &output)?;
    for change in changes {
        let field = checked
            .get(&change.name)
            .ok_or_else(|| BackendError::Malformed("repaired field disappeared".into()))?;
        if change
            .readonly
            .is_some_and(|value| field.is_readonly() != value)
            || change
                .required
                .is_some_and(|value| field.is_required() != value)
            || change
                .tooltip
                .as_ref()
                .is_some_and(|value| field.tooltip() != Some(value.as_str()))
            || change
                .max_length
                .is_some_and(|value| field.get_max_length() != Some(value))
            || change
                .alignment
                .is_some_and(|value| field.get_alignment() != Some(value))
            || change.rect.is_some_and(|value| {
                field_rectangle(field).is_none_or(|rect| {
                    rect.iter()
                        .zip(value)
                        .any(|(actual, expected)| (actual - expected).abs() > 0.01)
                })
            })
            || change.background_color.is_some_and(|value| {
                field.get_background_color().is_none_or(|actual| {
                    actual
                        .iter()
                        .zip(value)
                        .any(|(actual, expected)| (actual - expected).abs() > 0.0001)
                })
            })
            || change.border_color.is_some_and(|value| {
                field.get_border_color().is_none_or(|actual| {
                    actual
                        .iter()
                        .zip(value)
                        .any(|(actual, expected)| (actual - expected).abs() > 0.0001)
                })
            })
            || change.border_width.is_some_and(|value| {
                field
                    .get_border_width()
                    .is_none_or(|actual| (actual - value).abs() > 0.0001)
            })
        {
            return Err(BackendError::Unsupported(
                "AcroForm property repair did not round-trip exactly".into(),
            ));
        }
    }
    Ok(output)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoveParams {
    names: Vec<String>,
}

fn remove(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    document_scope(context)?;
    let params: RemoveParams = parse(params)?;
    bounded_count(
        params.names.len(),
        context.limits.max_items,
        "removed fields",
    )?;
    let mut editor = edit_document(context)?;
    let original = fields(context, &mut editor)?;
    for name in &params.names {
        let field = original
            .get(name)
            .ok_or_else(|| BackendError::Malformed(format!("form field does not exist: {name}")))?;
        if field.is_parent_only() || matches!(field.field_type(), Some(FieldType::Signature)) {
            return Err(BackendError::Unsupported(
                "parent containers and signature fields cannot be removed by this operation".into(),
            ));
        }
        editor.remove_form_field(name).map_err(upstream)?;
    }
    let output = saved(
        context,
        &mut editor,
        json!({"removed_fields": params.names}),
    )?;
    let output = repair_removed_fields(context, output, &params.names)?;
    let actual = verify_fields(context, &output)?;
    if params.names.iter().any(|name| actual.contains_key(name)) {
        return Err(BackendError::Unsupported(
            "pinned writer retained a field requested for removal".into(),
        ));
    }
    Ok(output)
}

/// The pinned editor tracks deleted names but its writer retains the original
/// AcroForm/Annots references. Unlink only exact field/widget object identities;
/// do not prune parent containers or claim that orphaned values were erased.
fn repair_removed_fields(
    context: &Context<'_>,
    output: OperationOutput,
    names: &[String],
) -> Result<OperationOutput, BackendError> {
    let pdf =
        pdf_oxide::PdfDocument::from_bytes(output.artifacts[0].bytes.clone()).map_err(upstream)?;
    super::security::validate_form_tree(&pdf, context)?;
    let extracted =
        pdf_oxide::extractors::forms::FormExtractor::extract_fields(&pdf).map_err(upstream)?;
    bounded_count(
        extracted.len(),
        context.limits.max_items,
        "serialized form fields",
    )?;
    let requested: HashSet<_> = names.iter().map(String::as_str).collect();
    let mut target_ids = HashSet::new();
    for field in extracted {
        if requested.contains(field.full_name.as_str()) {
            let reference = field.object_ref.ok_or_else(|| {
                BackendError::Unsupported(
                    "direct field removal requires an indirect field identity".into(),
                )
            })?;
            target_ids.insert((reference.id, reference.r#gen));
        }
    }
    if target_ids.is_empty() {
        return Ok(output);
    }
    let mut document =
        lopdf::Document::load_mem(&output.artifacts[0].bytes).map_err(repair_error)?;
    let mut deleted = target_ids.clone();
    for id in &target_ids {
        let field = document
            .get_object(*id)
            .map_err(repair_error)?
            .as_dict()
            .map_err(repair_error)?;
        if let Ok(kids) = field.get(b"Kids") {
            let kids = repair_object(&document, kids)?.1;
            let kids = kids.as_array().map_err(repair_error)?;
            bounded_count(
                kids.len().saturating_add(deleted.len()),
                context.limits.max_items,
                "removed widgets",
            )?;
            for kid in kids {
                let (widget_id, widget) = repair_object(&document, kid)?;
                let widget = widget.as_dict().map_err(repair_error)?;
                if widget.has(b"T")
                    || widget.has(b"Kids")
                    || widget.get(b"Subtype").and_then(lopdf::Object::as_name).ok()
                        != Some(b"Widget".as_slice())
                {
                    return Err(BackendError::Unsupported("removing a field with named or nested children requires explicit widget provenance".into()));
                }
                deleted.insert(widget_id.ok_or_else(|| {
                    BackendError::Unsupported(
                        "direct widget removal requires an indirect widget identity".into(),
                    )
                })?);
            }
        }
    }
    let root = document.trailer.get(b"Root").map_err(repair_error)?.clone();
    let (root_id, root) = repair_object(&document, &root)?;
    let root_id = root_id.ok_or_else(|| {
        BackendError::Unsupported("serialized catalog has no indirect identity".into())
    })?;
    let mut catalog = root.as_dict().map_err(repair_error)?.clone();
    let form = catalog.get(b"AcroForm").map_err(repair_error)?.clone();
    let (form_id, form) = repair_object(&document, &form)?;
    let mut form = form.as_dict().map_err(repair_error)?.clone();
    for id in &target_ids {
        let field = document
            .get_object(*id)
            .map_err(repair_error)?
            .as_dict()
            .map_err(repair_error)?;
        if let Ok(parent) = field.get(b"Parent") {
            let (parent_id, parent) = repair_object(&document, parent)?;
            let parent_id = parent_id.ok_or_else(|| {
                BackendError::Unsupported("field parent has no indirect identity".into())
            })?;
            let mut parent = parent.as_dict().map_err(repair_error)?.clone();
            let previous = parent.get(b"Kids").map_err(repair_error)?.clone();
            let (kept, removed) = without_references(
                &document,
                &previous,
                &HashSet::from([*id]),
                context.limits.max_items,
            )?;
            if removed != 1 {
                return Err(BackendError::Unsupported(
                    "field parent does not identify exactly one requested child".into(),
                ));
            }
            parent.set("Kids", kept);
            document
                .objects
                .insert(parent_id, lopdf::Object::Dictionary(parent));
        } else {
            let previous = form.get(b"Fields").map_err(repair_error)?.clone();
            let (kept, removed) = without_references(
                &document,
                &previous,
                &HashSet::from([*id]),
                context.limits.max_items,
            )?;
            if removed != 1 {
                return Err(BackendError::Unsupported(
                    "AcroForm does not identify exactly one requested root field".into(),
                ));
            }
            form.set("Fields", kept);
        }
    }
    if let Ok(order) = form.get(b"CO") {
        let (kept, _) = without_references(&document, order, &deleted, context.limits.max_items)?;
        form.set("CO", kept);
    }
    if let Some(form_id) = form_id {
        document
            .objects
            .insert(form_id, lopdf::Object::Dictionary(form));
    } else {
        catalog.set("AcroForm", form);
        document
            .objects
            .insert(root_id, lopdf::Object::Dictionary(catalog));
    }
    for (_, page_id) in document.get_pages() {
        let mut page = document
            .get_object(page_id)
            .map_err(repair_error)?
            .as_dict()
            .map_err(repair_error)?
            .clone();
        if let Ok(annotations) = page.get(b"Annots") {
            let (kept, removed) =
                without_references(&document, annotations, &deleted, context.limits.max_items)?;
            if removed != 0 {
                if kept.is_empty() {
                    page.remove(b"Annots");
                } else {
                    page.set("Annots", kept);
                }
                document
                    .objects
                    .insert(page_id, lopdf::Object::Dictionary(page));
            }
        }
    }
    let mut output = repaired_output(
        context,
        output,
        document,
        "unlink exact removed form/widget identities from Fields, parent Kids, calculation order and page Annots",
    )?;
    output.warnings.push("Form removal unlinks interactive fields and widgets; orphaned objects may retain old values, so this operation is not confidential data erasure.".into());
    Ok(output)
}

fn without_references(
    document: &lopdf::Document,
    object: &lopdf::Object,
    removed_ids: &HashSet<lopdf::ObjectId>,
    maximum: usize,
) -> Result<(Vec<lopdf::Object>, usize), BackendError> {
    let object = repair_object(document, object)?.1;
    let array = object.as_array().map_err(repair_error)?;
    bounded_count(array.len(), maximum, "form/widget reference array")?;
    let mut kept = Vec::with_capacity(array.len());
    let mut removed = 0;
    for value in array {
        let id = repair_object(document, value)?.0;
        if id.is_some_and(|id| removed_ids.contains(&id)) {
            removed += 1;
        } else {
            kept.push(value.clone());
        }
    }
    Ok((kept, removed))
}

fn flatten(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    let _: Empty = parse(params)?;
    let mut editor = edit_document(context)?;
    let original = fields(context, &mut editor)?;
    if original
        .values()
        .any(|field| matches!(field.field_type(), Some(FieldType::Signature)))
    {
        return Err(BackendError::Unsupported(
            "signature widgets are not flattened".into(),
        ));
    }
    reject_pending_appearance_flatten(context, &editor)?;
    if context.pages.is_none() {
        editor.flatten_forms().map_err(upstream)?;
    } else {
        for page in context.selected_pages(editor.current_page_count())? {
            editor.flatten_forms_on_page(page).map_err(upstream)?;
        }
    }
    let mut output = saved(context, &mut editor, json!({"flattened": true}))?;
    output.warnings.push("Flattening depends on widget appearances; any missing appearance is an omission, not proof of visually complete form rendering.".into());
    if context.pages.is_none() && !verify_fields(context, &output)?.is_empty() {
        return Err(BackendError::Unsupported(
            "pinned writer retained fields after full form flattening".into(),
        ));
    }
    Ok(output)
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ExportFormat {
    Fdf,
    Xfdf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportParams {
    format: ExportFormat,
    /// Respect `/NoExport` by default.
    #[serde(default)]
    include_no_export: bool,
}

fn fdf_value(value: FormFieldValue) -> FdfValue {
    match value {
        FormFieldValue::Text(value) => FdfValue::Text(value),
        FormFieldValue::Boolean(value) => FdfValue::Boolean(value),
        FormFieldValue::Choice(value) => FdfValue::Name(value),
        FormFieldValue::MultiChoice(value) => FdfValue::Array(value),
        FormFieldValue::None => FdfValue::None,
    }
}

fn xml_safe(text: &str) -> bool {
    text.chars()
        .all(|c| matches!(c, '\t' | '\n' | '\r') || c >= ' ' && c != '\u{fffe}' && c != '\u{ffff}')
}

fn export(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    document_scope(context)?;
    let params: ExportParams = parse(params)?;
    let mut editor = DocumentEditor::from_document(context.open_pdf()?).map_err(upstream)?;
    bounded_count(
        editor.current_page_count(),
        context.limits.max_pages as usize,
        "form export source pages",
    )?;
    let fields = fields(context, &mut editor)?;
    let mut fdf = FdfWriter::new();
    let mut xfdf = XfdfWriter::new();
    let mut exported = Vec::new();
    let mut skipped = Vec::new();
    for field in fields.values() {
        if field.is_parent_only() || !params.include_no_export && field.is_no_export() {
            skipped.push(field.name().to_owned());
            continue;
        }
        let value = field.value();
        let values: Vec<&str> = match &value {
            FormFieldValue::Text(text) | FormFieldValue::Choice(text) => vec![text],
            FormFieldValue::MultiChoice(values) => values.iter().map(String::as_str).collect(),
            _ => Vec::new(),
        };
        bounded_count(
            values.len(),
            context.limits.max_items,
            "export field values",
        )?;
        for value in &values {
            bounded_string(value, "export field value")?;
        }
        if !xml_safe(field.name()) || values.iter().any(|value| !xml_safe(value)) {
            return Err(BackendError::Unsupported(
                "field names or values contain characters forbidden in XML 1.0".into(),
            ));
        }
        match params.format {
            ExportFormat::Fdf => {
                if !field.name().is_ascii() || values.iter().any(|value| !value.is_ascii()) {
                    return Err(BackendError::Unsupported(
                        "pinned FDF text writer does not encode Unicode PDF strings; use XFDF"
                            .into(),
                    ));
                }
                if let FormFieldValue::Choice(choice) = &value
                    && !choice
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                {
                    return Err(BackendError::Unsupported(
                        "pinned FDF writer does not escape PDF Name values; use XFDF".into(),
                    ));
                }
                fdf.add_field(FdfField::new(field.name(), fdf_value(value)));
            }
            ExportFormat::Xfdf => {
                if matches!(value, FormFieldValue::MultiChoice(_)) {
                    return Err(BackendError::Unsupported("pinned XFDF writer comma-joins multi-values; use FDF for lossless ASCII arrays".into()));
                }
                xfdf.add_fdf_field(FdfField::new(field.name(), fdf_value(value)));
            }
        }
        exported.push(field.name().to_owned());
    }
    let (name, mime, bytes) = match params.format {
        ExportFormat::Fdf => (
            "form-data.fdf",
            "application/vnd.fdf",
            fdf.to_bytes().map_err(upstream)?,
        ),
        ExportFormat::Xfdf => (
            "form-data.xfdf",
            "application/vnd.adobe.xfdf",
            xfdf.to_bytes(),
        ),
    };
    if bytes.len() as u64 > context.limits.max_output_bytes {
        return Err(BackendError::Limit(
            "form export exceeds output byte budget".into(),
        ));
    }
    Ok(OperationOutput {
        data: json!({"exported_fields": exported, "skipped_fields": skipped}),
        artifacts: vec![Artifact {
            name: name.into(),
            mime: mime.into(),
            bytes,
        }],
        warnings: Vec::new(),
    })
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum XfaParams {
    Inspect,
    ConvertStatic { accept_layout_loss: bool },
}

fn validate_xfa_xml(bytes: &[u8], context: &Context<'_>) -> Result<(), BackendError> {
    if bytes.len() as u64 > context.limits.max_input_bytes {
        return Err(BackendError::Limit(
            "decoded XFA exceeds input byte budget".into(),
        ));
    }
    let mut reader = Reader::from_reader(bytes);
    let mut depth = 0_usize;
    let mut count = 0_usize;
    loop {
        let event = reader
            .read_event()
            .map_err(|e| BackendError::Malformed(format!("XFA XML: {e}")))?;
        if matches!(event, Event::Start(_)) {
            depth += 1;
            bounded_count(depth, 32, "XFA XML depth")?;
        }
        match event {
            Event::Start(tag) | Event::Empty(tag) => {
                count += 1;
                bounded_count(
                    count,
                    context.limits.max_items.saturating_mul(16),
                    "XFA XML elements",
                )?;
                let name = tag.local_name();
                if [
                    b"script".as_slice(),
                    b"calculate",
                    b"event",
                    b"connectionSet",
                    b"connect",
                    b"submit",
                    b"execute",
                    b"occur",
                ]
                .contains(&name.as_ref())
                {
                    return Err(BackendError::Unsupported("dynamic XFA scripts, actions, connections and repeated sections are not supported".into()));
                }
                for attribute in tag.attributes() {
                    let attribute = attribute
                        .map_err(|e| BackendError::Malformed(format!("XFA attribute: {e}")))?;
                    if [b"href".as_slice(), b"src", b"usehref"]
                        .contains(&attribute.key.local_name().as_ref())
                    {
                        return Err(BackendError::Unsupported(
                            "XFA resource references are not supported".into(),
                        ));
                    }
                }
            }
            Event::DocType(_) => {
                return Err(BackendError::Unsupported(
                    "XFA DTDs are not supported".into(),
                ));
            }
            Event::GeneralRef(reference) => {
                let standard = [b"amp".as_slice(), b"lt", b"gt", b"quot", b"apos"]
                    .contains(&reference.as_ref());
                let character = reference.resolve_char_ref().map_err(|e| {
                    BackendError::Malformed(format!("XFA character reference: {e}"))
                })?;
                if !standard && character.is_none() {
                    return Err(BackendError::Unsupported(
                        "XFA custom entity references are not supported".into(),
                    ));
                }
            }
            Event::End(_) => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| BackendError::Malformed("unbalanced XFA XML".into()))?;
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if depth != 0 {
        return Err(BackendError::Malformed("unclosed XFA XML elements".into()));
    }
    Ok(())
}

fn xfa(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    document_scope(context)?;
    let params: XfaParams = parse(params)?;
    let mut doc = context.open_pdf()?;
    bounded_count(
        doc.page_count().map_err(upstream)?,
        context.limits.max_pages as usize,
        "XFA source pages",
    )?;
    if !XfaExtractor::has_xfa(&mut doc).map_err(upstream)? {
        return match params {
            XfaParams::Inspect => Ok(OperationOutput {
                data: json!({"has_xfa": false}),
                ..Default::default()
            }),
            XfaParams::ConvertStatic { .. } => {
                Err(BackendError::Malformed("source has no XFA form".into()))
            }
        };
    }
    let bytes = XfaExtractor::extract_xfa(&mut doc).map_err(upstream)?;
    validate_xfa_xml(&bytes, context)?;
    let form = XfaParser::new().parse(&bytes).map_err(upstream)?;
    bounded_count(form.field_count(), context.limits.max_items, "XFA fields")?;
    bounded_count(
        form.pages.len(),
        context.limits.max_pages as usize,
        "XFA pages",
    )?;
    let records = form
        .fields
        .iter()
        .map(|field| {
            json!({"name": field.name,
        "binding": field.binding, "type": format!("{:?}", field.field_type), "value": field.value,
        "required": field.required, "readonly": field.readonly})
        })
        .collect::<Vec<_>>();
    match params {
        XfaParams::Inspect => Ok(OperationOutput {
            data: json!({"has_xfa": true, "fields": records}),
            ..Default::default()
        }),
        XfaParams::ConvertStatic { accept_layout_loss } => {
            if !accept_layout_loss {
                return Err(BackendError::Unsupported("static XFA conversion creates a new vertically stacked layout; accept_layout_loss must be true".into()));
            }
            // Apply the same encrypted/signed-source edit policy as other derivatives.
            drop(edit_document(context)?);
            let result = XfaConverter::new().convert(&form).map_err(upstream)?;
            bounded_count(
                result.pages.len(),
                context.limits.max_pages as usize,
                "converted XFA pages",
            )?;
            if !result.skipped_fields.is_empty() {
                return Err(BackendError::Unsupported(format!(
                    "XFA converter omitted {} fields",
                    result.skipped_fields.len()
                )));
            }
            let mut writer = PdfWriter::new();
            for converted in &result.pages {
                let mut page = writer.add_page(converted.width, converted.height);
                add_converted_page(&mut page, converted);
            }
            let bytes = writer.finish().map_err(upstream)?;
            Ok(OperationOutput {
                data: json!({"converted_fields": result.field_count, "source_fields": records}),
                artifacts: vec![pdf_artifact(context, "static-acroform.pdf".into(), bytes, result.pages.len())?],
                warnings: vec!["Static XFA conversion creates a new document with a simplified vertically stacked layout; original visual layout and non-form content are not retained.".into()],
            })
        }
    }
}
