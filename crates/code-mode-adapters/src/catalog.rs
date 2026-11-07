//! Named adapters over the vendored official OpenAPI documents.
//!
//! Sources, versions, and SHA-256 checksums are in `specs/vendor/SOURCES.json`.

use crate::adapter::{AdapterError, ApiAdapter, OpenApiAdapter};

const GITHUB_SPEC: &str = include_str!("../../../specs/vendor/github-rest-openapi.json");
const VERCEL_SPEC: &str = include_str!("../../../specs/vendor/vercel-rest-openapi.json");
const SUPABASE_SPEC: &str = include_str!("../../../specs/vendor/supabase-management-openapi.json");

#[derive(Debug)]
pub struct GitHubAdapter {
    inner: OpenApiAdapter,
}

impl GitHubAdapter {
    pub fn new(
        token: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Result<Self, AdapterError> {
        Ok(Self {
            inner: bearer(
                "github",
                "GITHUB_TOKEN",
                token,
                base_url,
                GITHUB_SPEC,
                vec![
                    ("Accept".into(), "application/vnd.github+json".into()),
                    ("X-GitHub-Api-Version".into(), "2022-11-28".into()),
                ],
            )?,
        })
    }
}

#[derive(Debug)]
pub struct VercelAdapter {
    inner: OpenApiAdapter,
}

impl VercelAdapter {
    pub fn new(
        token: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Result<Self, AdapterError> {
        Ok(Self {
            inner: bearer(
                "vercel",
                "VERCEL_TOKEN",
                token,
                base_url,
                VERCEL_SPEC,
                Vec::new(),
            )?,
        })
    }
}

#[derive(Debug)]
pub struct SupabaseAdapter {
    inner: OpenApiAdapter,
}

impl SupabaseAdapter {
    pub fn new(
        token: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Result<Self, AdapterError> {
        Ok(Self {
            inner: bearer(
                "supabase",
                "SUPABASE_ACCESS_TOKEN",
                token,
                base_url,
                SUPABASE_SPEC,
                Vec::new(),
            )?,
        })
    }
}

fn bearer(
    id: &str,
    token_env: &str,
    token: impl Into<String>,
    base_url: impl Into<String>,
    spec: &str,
    extra: Vec<(String, String)>,
) -> Result<OpenApiAdapter, AdapterError> {
    let token = token.into();
    if token.is_empty() {
        return Err(AdapterError::MissingCredential(token_env.into()));
    }
    OpenApiAdapter::new(id, base_url, spec, Some(token), extra)
}

macro_rules! delegate {
    ($ty:ty) => {
        impl ApiAdapter for $ty {
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
    };
}

delegate!(GitHubAdapter);
delegate!(VercelAdapter);
delegate!(SupabaseAdapter);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_sends_version_and_accept() {
        let adapter = GitHubAdapter::new("ghp_test", "https://api.github.com").unwrap();
        let headers = adapter.headers().unwrap();
        assert!(headers
            .iter()
            .any(|(name, value)| name == "Authorization" && value == "Bearer ghp_test"));
        assert!(headers
            .iter()
            .any(|(name, value)| { name == "Accept" && value == "application/vnd.github+json" }));
        assert!(headers
            .iter()
            .any(|(name, value)| { name == "X-GitHub-Api-Version" && value == "2022-11-28" }));
        assert_eq!(
            headers
                .iter()
                .filter(|(name, _)| name.eq_ignore_ascii_case("accept"))
                .count(),
            1
        );
    }

    #[test]
    fn empty_tokens_name_the_env_var() {
        assert!(GitHubAdapter::new("", "https://api.github.com")
            .unwrap_err()
            .to_string()
            .contains("GITHUB_TOKEN"));
        assert!(VercelAdapter::new("", "https://api.vercel.com")
            .unwrap_err()
            .to_string()
            .contains("VERCEL_TOKEN"));
        assert!(SupabaseAdapter::new("", "https://api.supabase.com")
            .unwrap_err()
            .to_string()
            .contains("SUPABASE_ACCESS_TOKEN"));
    }
}
