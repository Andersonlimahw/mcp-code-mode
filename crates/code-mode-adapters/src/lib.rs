//! API adapters.
//!
//! [`ApiAdapter`] is the extension point. [`OpenApiAdapter`] is the generic
//! bearer-token implementation driven by an OpenAPI document. [`NotionAdapter`]
//! is the pilot: Notion-Version header plus the embedded 2022-06-28 spec.

mod adapter;
mod catalog;
mod http;
mod invoke;
mod notion;

pub use adapter::{AdapterError, ApiAdapter, OpenApiAdapter};
pub use catalog::{GitHubAdapter, SupabaseAdapter, VercelAdapter};
pub use http::{cache_key, HttpError, HttpRequest, HttpResponse, HttpTransport, UreqTransport};
pub use invoke::{EndpointParts, HttpInvoker, InvokerParts};
pub use notion::NotionAdapter;
