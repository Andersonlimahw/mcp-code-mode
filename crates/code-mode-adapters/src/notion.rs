use crate::adapter::{AdapterError, ApiAdapter, OpenApiAdapter};

/// Notion public REST API, version header defaulting to `2022-06-28`.
///
/// That version still exposes `POST /v1/databases` and
/// `PATCH /v1/databases/{database_id}`. The embedded spec covers the pilot
/// operations: search, create page, update page, update block, append block
/// children, create database, update database.
#[derive(Debug)]
pub struct NotionAdapter {
    inner: OpenApiAdapter,
}

const SPEC: &str = include_str!("../../../specs/notion-2022-06-28.json");

impl NotionAdapter {
    pub fn new(
        token: impl Into<String>,
        base_url: impl Into<String>,
        notion_version: impl Into<String>,
    ) -> Result<Self, AdapterError> {
        let token = token.into();
        if token.is_empty() {
            return Err(AdapterError::MissingCredential("NOTION_TOKEN".into()));
        }
        let version = notion_version.into();
        let inner = OpenApiAdapter::new(
            "notion",
            base_url,
            SPEC,
            Some(token),
            vec![("Notion-Version".into(), version)],
        )?;
        Ok(Self { inner })
    }

    pub fn with_spec(mut self, spec_json: impl Into<String>) -> Self {
        self.inner = self.inner.with_spec(spec_json);
        self
    }

    pub fn push_header(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.inner.push_header(name, value);
    }
}

impl ApiAdapter for NotionAdapter {
    fn id(&self) -> &str {
        self.inner.id()
    }

    fn base_url(&self) -> &str {
        self.inner.base_url()
    }

    fn spec_json(&self) -> &str {
        self.inner.spec_json()
    }

    fn headers(&self) -> Result<Vec<(String, String)>, AdapterError> {
        self.inner.headers()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notion_headers_include_bearer_and_version() {
        let adapter =
            NotionAdapter::new("secret", "https://api.notion.com/", "2022-06-28").unwrap();
        assert_eq!(adapter.base_url(), "https://api.notion.com");
        let headers = adapter.headers().unwrap();
        assert!(headers
            .iter()
            .any(|(name, value)| name == "Authorization" && value == "Bearer secret"));
        assert!(headers
            .iter()
            .any(|(name, value)| name == "Notion-Version" && value == "2022-06-28"));
        assert!(adapter.spec_json().contains("create_database"));
    }

    #[test]
    fn empty_token_is_rejected() {
        let error = NotionAdapter::new("", "https://api.notion.com", "2022-06-28").unwrap_err();
        assert!(error.to_string().contains("NOTION_TOKEN"));
    }
}
