use std::borrow::Cow;
use std::fmt;

#[cfg(test)]
use httpmock::MockServer;
use reqwest::header::HeaderMap;
use reqwest::{Client, Url};
use serde::de::DeserializeOwned;
use tracing::{debug, info, instrument};

use crate::model::{app, node, table};

pub type ClientResult<T> = Result<T, ClientError>;

#[derive(Clone)]
pub struct TiledClient {
    client: Client,
    address: Url,
}

impl TiledClient {
    pub fn new(address: Url) -> Self {
        if address.cannot_be_a_base() {
            // Panicking is not great but if we've got this far, nothing else is going to work so
            // bail out early.
            panic!("Invalid tiled URL");
        }
        Self {
            client: Client::new(),
            address,
        }
    }
    #[instrument(skip(self))]
    async fn request<T: DeserializeOwned>(
        &self,
        endpoint: &str,
        headers: Option<HeaderMap>,
        query_params: Option<&[(&str, Cow<'_, str>)]>,
    ) -> ClientResult<T> {
        let url = self.address.join(endpoint)?;

        let mut request = match headers {
            Some(headers) => self.client.get(url).headers(headers),
            None => self.client.get(url),
        };
        if let Some(params) = query_params {
            request = request.query(&params);
        }
        info!("Querying: {request:?}");

        let response = request.send().await?;
        let status = response.status().as_u16();
        let body = response.text().await?;
        match status {
            400..500 => Err(ClientError::TiledRequest(status, body)),
            500..600 => Err(ClientError::TiledInternal(status, body)),
            _ => serde_json::from_str(&body).map_err(|e| ClientError::InvalidResponse(e, body)),
        }
    }
    pub async fn app_metadata(&self) -> ClientResult<app::AppMetadata> {
        self.request("/api/v1/", None, None).await
    }
    pub async fn search(
        &self,
        path: &str,
        headers: Option<HeaderMap>,
        query: &[(&str, Cow<'_, str>)],
    ) -> ClientResult<node::Root> {
        self.request(&format!("api/v1/search/{}", path), headers, Some(query))
            .await
    }

    pub async fn metadata(
        &self,
        id: String,
        headers: Option<HeaderMap>,
    ) -> ClientResult<node::Metadata> {
        self.request(&format!("api/v1/metadata/{id}"), headers, None)
            .await
    }

    pub async fn table_full(
        &self,
        path: &str,
        columns: Option<Vec<String>>,
        headers: Option<HeaderMap>,
    ) -> ClientResult<table::Table> {
        let mut headers = headers.unwrap_or_default();
        headers.insert("accept", "application/json".parse().unwrap());
        let query = columns.map(|columns| {
            columns
                .into_iter()
                .map(|col| ("column", col.into()))
                .collect::<Vec<_>>()
        });

        self.request(
            &format!("/api/v1/table/full/{}", path),
            Some(headers),
            query.as_deref(),
        )
        .await
    }

    pub(crate) async fn download(
        &self,
        run: String,
        stream: String,
        det: String,
        id: u32,
        headers: Option<HeaderMap>,
    ) -> reqwest::Result<reqwest::Response> {
        let mut url = self
            .address
            .join("/api/v1/asset/bytes")
            .expect("Base address was cannot_be_a_base");
        url.path_segments_mut()
            .expect("Base address was cannot_be_a_base")
            .push(&run)
            .push(&stream)
            .push(&det);

        debug!("Downloading id={id} from {url}");
        self.client
            .get(url)
            .headers(headers.unwrap_or_default())
            .query(&[("id", &id.to_string())])
            .send()
            .await
    }

    /// Create a new client for the given mock server
    #[cfg(test)]
    pub fn for_mock_server(server: &MockServer) -> Self {
        Self {
            // We're only in tests so panicking is fine
            address: server.base_url().parse().unwrap(),
            client: Client::new(),
        }
    }
}

#[derive(Debug)]
pub enum ClientError {
    InvalidPath(url::ParseError),
    ServerError(reqwest::Error),
    InvalidResponse(serde_json::Error, String),
    TiledInternal(u16, String),
    TiledRequest(u16, String),
}
impl From<url::ParseError> for ClientError {
    fn from(err: url::ParseError) -> ClientError {
        ClientError::InvalidPath(err)
    }
}
impl From<reqwest::Error> for ClientError {
    fn from(err: reqwest::Error) -> ClientError {
        ClientError::ServerError(err)
    }
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ClientError::InvalidPath(err) => write!(f, "Invalid URL path: {}", err),
            ClientError::ServerError(err) => write!(f, "Tiled server error: {}", err),
            ClientError::TiledInternal(sc, message) => {
                write!(f, "Internal tiled error: {sc} - {message}")
            }
            ClientError::TiledRequest(sc, message) => {
                write!(f, "Request Error: {sc} - {message}")
            }
            ClientError::InvalidResponse(err, actual) => {
                write!(f, "Invalid response: {err}, response: {actual}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderMap;
    use httpmock::MockServer;

    use crate::clients::{ClientError, TiledClient};

    #[tokio::test]
    async fn request() {
        let server = MockServer::start();
        let mock = server
            .mock_async(|when, then| {
                when.method("GET").path("/demo/api");
                then.status(200).body("[1,2,3]");
            })
            .await;
        let client = TiledClient::for_mock_server(&server);
        assert_eq!(
            client
                .request::<Vec<u8>>("/demo/api", None, None)
                .await
                .unwrap(),
            vec![1, 2, 3]
        );
        mock.assert();
    }
    #[tokio::test]
    async fn request_with_headers() {
        let server = MockServer::start();
        let mock = server
            .mock_async(|when, then| {
                when.method("GET")
                    .path("/demo/api")
                    .header("api-key", "foo");
                then.status(200).body("[1,2,3]");
            })
            .await;
        let client = TiledClient::for_mock_server(&server);
        let mut headers = HeaderMap::new();
        headers.insert("api-key", "foo".parse().unwrap());

        assert_eq!(
            client
                .request::<Vec<u8>>("/demo/api", Some(headers), None)
                .await
                .unwrap(),
            vec![1, 2, 3]
        );
        mock.assert();
    }

    #[tokio::test]
    async fn request_app_metadata() {
        let server = MockServer::start();
        let mock = server
            .mock_async(|when, then| {
                when.method("GET").path("/api/v1/");
                then.status(200)
                    .body_from_file("resources/metadata_app.json");
            })
            .await;
        let client = TiledClient::for_mock_server(&server);
        let response = client.app_metadata().await.unwrap();

        assert_eq!(response.api_version, 0);
        mock.assert();
    }
    #[tokio::test]
    async fn server_unavailable() {
        let client = TiledClient::new("http://non-existent.example.com".parse().unwrap());
        let response = client.app_metadata().await;

        let Err(ClientError::ServerError(err)) = response else {
            panic!("Expected ServerError but got {response:?}");
        };
        assert!(
            err.is_connect(),
            "Expected connection error but got {err:?}"
        );
    }

    #[tokio::test]
    async fn internal_tiled_error() {
        let server = MockServer::start();
        let mock = server
            .mock_async(|when, then| {
                when.method("GET").path("/api/v1/");
                then.status(503).body("Tiled is broken inside");
            })
            .await;

        let client = TiledClient::for_mock_server(&server);
        let response = client.app_metadata().await;

        let Err(ClientError::TiledInternal(503, err)) = response else {
            panic!("Expected ServerError but got {response:?}");
        };

        assert_eq!(err, "Tiled is broken inside");

        mock.assert();
    }

    #[tokio::test]
    async fn invalid_server_response() {
        let server = MockServer::start();
        let mock = server
            .mock_async(|when, then| {
                when.method("GET").path("/api/v1/");
                then.status(200).body("{}");
            })
            .await;

        let client = TiledClient::for_mock_server(&server);
        let response = client.app_metadata().await;

        let Err(ClientError::InvalidResponse(err, _)) = response else {
            panic!("Expected InvalidResponse but got {response:?}");
        };

        assert!(err.is_data());
        mock.assert();
    }
}
