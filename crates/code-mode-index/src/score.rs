use code_mode_core::Operation;

/// Split a user query into lowercase alphanumeric terms, preserving order.
pub fn query_terms(query: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut current = String::new();
    for ch in query.chars() {
        if ch.is_ascii_alphanumeric() {
            current.push(ch.to_ascii_lowercase());
        } else if !current.is_empty() {
            push_term(&mut terms, &current);
            current.clear();
        }
    }
    if !current.is_empty() {
        push_term(&mut terms, &current);
    }
    terms
}

fn push_term(terms: &mut Vec<String>, term: &str) {
    if !terms.iter().any(|existing| existing == term) {
        terms.push(term.to_string());
    }
}

/// Higher is a closer match. Zero means the operation does not match.
pub fn lexical_score(operation: &Operation, terms: &[String]) -> i32 {
    if terms.is_empty() {
        return 0;
    }
    let id = spaced(&operation.operation_id);
    let binding = spaced(&operation.binding_name);
    let path = operation.path.to_lowercase();
    let summary = operation.summary.to_lowercase();
    let description = operation.description.to_lowercase();
    let tags = operation.tags.join(" ").to_lowercase();
    let mut score = 0;
    for term in terms {
        if token_eq(&id, term) || token_eq(&binding, term) {
            score += 50;
        } else if id.contains(term) || binding.contains(term) {
            score += 20;
        }
        if path.contains(term) {
            score += 15;
        }
        if summary.contains(term) {
            score += 25;
        }
        if tags.contains(term) {
            score += 10;
        }
        if description.contains(term) {
            score += 5;
        }
    }
    score
}

fn spaced(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(' ');
        }
    }
    out
}

fn token_eq(haystack: &str, term: &str) -> bool {
    haystack.split_whitespace().any(|word| word == term)
}

/// Document stored in FTS. Identifier separators become spaces so "create page"
/// matches `create_page`.
pub fn fts_document(operation: &Operation) -> String {
    format!(
        "{} {} {} {} {} {}",
        spaced(&operation.operation_id),
        spaced(&operation.binding_name),
        operation.path.replace(['/', '{', '}'], " "),
        operation.summary,
        operation.description,
        operation.tags.join(" ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use code_mode_core::{Operation, Parameter};
    use serde_json::json;

    fn op(id: &str, path: &str, summary: &str, description: &str) -> Operation {
        Operation {
            operation_id: id.into(),
            binding_name: id.into(),
            method: "POST".into(),
            path: path.into(),
            summary: summary.into(),
            description: description.into(),
            tags: vec!["Pages".into()],
            parameters: Vec::<Parameter>::new(),
            request_schema: json!({}),
            response_schema: json!({}),
        }
    }

    #[test]
    fn terms_split_and_dedup() {
        assert_eq!(
            query_terms("Create page, create"),
            vec!["create".to_string(), "page".to_string()]
        );
    }

    #[test]
    fn create_page_outranks_update_page() {
        let terms = query_terms("create page");
        let create = op("create_page", "/v1/pages", "Create a page", "Create a page");
        let update = op(
            "update_page",
            "/v1/pages/{page_id}",
            "Update a page",
            "Update page properties",
        );
        assert!(lexical_score(&create, &terms) > lexical_score(&update, &terms));
    }
}
