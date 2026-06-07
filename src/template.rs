pub(crate) fn render(template: &str, values: &[(&str, String)]) -> String {
    for placeholder in placeholders(template) {
        assert!(
            values.iter().any(|(key, _)| *key == placeholder),
            "template placeholder {{{{{placeholder}}}}} was not provided"
        );
    }

    let mut output = template.to_string();
    for (key, value) in values {
        output = output.replace(&format!("{{{{{key}}}}}"), value);
    }
    output
}

fn placeholders(template: &str) -> Vec<&str> {
    let mut placeholders = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find("}}") else {
            break;
        };
        let placeholder = &rest[..end];
        if placeholder
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            placeholders.push(placeholder);
        }
        rest = &rest[end + 2..];
    }
    placeholders
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_replaces_known_placeholders() {
        assert_eq!(
            render("Hello {{name}}", &[("name", "George".into())]),
            "Hello George"
        );
    }

    #[test]
    #[should_panic(expected = "template placeholder {{name}} was not provided")]
    fn render_rejects_missing_placeholders() {
        render("Hello {{name}}", &[]);
    }

    #[test]
    fn render_allows_placeholder_text_inside_values() {
        assert_eq!(
            render("{{content}}", &[("content", "Literal {{example}}".into())]),
            "Literal {{example}}"
        );
    }
}
