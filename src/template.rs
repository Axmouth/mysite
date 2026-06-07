pub(crate) fn render(template: &str, values: &[(&str, String)]) -> String {
    let mut output = template.to_string();
    for (key, value) in values {
        output = output.replace(&format!("{{{{{key}}}}}"), value);
    }
    output
}
