use thiserror::Error;

/// Auth and spec source for one upstream API.
///
/// A new API is either another [`OpenApiAdapter`] configuration or a new type
/// that implements this trait when the HTTP conversation is not plain JSON.
pub trait ApiAdapter: Send + Sync {
    fn id(&self) -> &str;
    fn base_url(&self) -> &str;
    fn spec_json(&self) -> &str;
    fn headers(&self) -> Result<Vec<(String, String)>, AdapterError>;
}

#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("missing credential in {0}")]
    MissingCredential(String),
    #[error("{0}")]
    Message(String),
}

/// Generic OpenAPI-backed adapter. Operation routing lives in the invoker,
/// not here, so this type stays a description of the remote API.
#[derive(Debug, Clone)]
pub struct OpenApiAdapter {
    id: String,
    base_url: String,
    spec_json: String,
    bearer_token: Option<String>,
    extra_headers: Vec<(String, String)>,
}

impl OpenApiAdapter {
    pub fn new(
        id: impl Into<String>,
        base_url: impl Into<String>,
        spec_json: impl Into<String>,
        bearer_token: Option<String>,
        extra_headers: Vec<(String, String)>,
    ) -> Result<Self, AdapterError> {
        let id = id.into();
        if id.is_empty() {
            return Err(AdapterError::Message("adapter id is empty".into()));
        }
        let base_url = base_url.into().trim_end_matches('/').to_string();
        if base_url.is_empty() {
            return Err(AdapterError::Message("base url is empty".into()));
        }
        let spec_json = spec_json.into();
        if spec_json.trim().is_empty() {
            return Err(AdapterError::Message("OpenAPI document is empty".into()));
        }
        if let Some(token) = &bearer_token {
            if token.is_empty() {
                return Err(AdapterError::MissingCredential(
                    "bearer token is empty".into(),
                ));
            }
        }
        Ok(Self {
            id,
            base_url,
            spec_json,
            bearer_token,
            extra_headers,
        })
    }

    pub fn with_spec(mut self, spec_json: impl Into<String>) -> Self {
        self.spec_json = spec_json.into();
        self
    }

    pub fn push_header(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.extra_headers.push((name.into(), value.into()));
    }
}

impl ApiAdapter for OpenApiAdapter {
    fn id(&self) -> &str {
        &self.id
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn spec_json(&self) -> &str {
        &self.spec_json
    }

    fn headers(&self) -> Result<Vec<(String, String)>, AdapterError> {
        let mut headers = Vec::new();
        if let Some(token) = &self.bearer_token {
            headers.push(("Authorization".into(), format!("Bearer {token}")));
        }
        headers.extend(self.extra_headers.iter().cloned());
        let has_accept = headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("accept"));
        if !has_accept {
            headers.push(("Accept".into(), "application/json".into()));
        }
        Ok(headers)
    }
}
