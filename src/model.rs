pub(crate) mod app;
pub(crate) mod array;
pub(crate) mod container;
pub(crate) mod event_stream;
pub(crate) mod node;
pub(crate) mod run;
pub(crate) mod table;

use std::collections::HashMap;

use async_graphql::{Context, Object, Result, Union};
use serde_json::Value;
use tracing::{info, instrument};

use crate::RootAddress;
use crate::clients::TiledClient;
use crate::handlers::AuthHeader;
use crate::model::node::NodeAttributes;

pub(crate) struct TiledQuery;

#[Object]
impl TiledQuery {
    #[instrument(skip(self, ctx))]
    async fn app_metadata(&self, ctx: &Context<'_>) -> Result<app::AppMetadata> {
        Ok(ctx.data::<TiledClient>()?.app_metadata().await?)
    }

    async fn instrument_session(&self, name: String) -> InstrumentSession {
        InstrumentSession { name }
    }
}

struct InstrumentSession {
    name: String,
}

#[Object]
impl InstrumentSession {
    async fn name(&self) -> &str {
        &self.name
    }
    async fn runs(&self, ctx: &Context<'_>) -> Result<Vec<Run>> {
        let auth = ctx.data::<Option<AuthHeader>>()?;
        let headers = auth.as_ref().map(AuthHeader::as_header_map);
        let root = ctx
            .data::<TiledClient>()?
            .search(
                "",
                headers,
                &[
                    (
                        "filter[eq][condition][key]",
                        "start.instrument_session".into(),
                    ),
                    (
                        "filter[eq][condition][value]",
                        format!(r#""{}""#, self.name).into(),
                    ),
                    ("include_data_sources", "true".into()),
                ],
            )
            .await?;
        Ok(root.into_data().map(|d| Run { data: d }).collect())
    }
}

#[derive(Union)]
enum RunData<'run> {
    Array(ArrayData<'run>),
    Internal(TableData),
}

struct ArrayData<'run> {
    run: &'run Run,
    id: String,
    stream: String,
    attrs: node::Attributes<HashMap<String, Value>, array::ArrayStructure>,
}

#[Object]
impl<'run> ArrayData<'run> {
    async fn name(&self) -> &str {
        &self.id
    }
    async fn files<'ad>(&'ad self) -> Vec<Asset<'ad>> {
        self.attrs
            .data_sources
            .as_deref()
            .unwrap_or_default()
            .iter()
            .flat_map(|source| source.assets.iter())
            .map(|a| Asset {
                data: self,
                asset: a,
            })
            .collect()
    }
}

struct Asset<'a> {
    asset: &'a node::Asset,
    data: &'a ArrayData<'a>,
}

#[Object]
impl Asset<'_> {
    async fn file(&self) -> &str {
        &self.asset.data_uri
    }
    async fn download(&self, ctx: &Context<'_>) -> Option<String> {
        let id = self.asset.id?;
        let mut download = ctx.data::<RootAddress>().ok()?.0.clone();
        download
            .path_segments_mut()
            .ok()?
            .push("asset")
            .push(&self.data.run.data.id)
            .push(&self.data.stream)
            .push(&self.data.id)
            .push(&id.to_string());
        Some(download.to_string())
    }
}

struct TableData {
    id: String,
    attrs: node::Attributes<HashMap<String, Value>, table::TableStructure>,
}

#[Object]
impl TableData {
    async fn name(&self) -> &str {
        &self.id
    }
    async fn columns(&self) -> &[String] {
        &self.attrs.structure.columns
    }
    async fn data(
        &self,
        ctx: &Context<'_>,
        columns: Option<Vec<String>>,
    ) -> Option<Result<HashMap<String, Vec<Value>>>> {
        Some(self.inner_data(ctx, columns).await)
    }
}

impl TableData {
    async fn inner_data(
        &self,
        ctx: &Context<'_>,
        columns: Option<Vec<String>>,
    ) -> Result<HashMap<String, Vec<Value>>> {
        let auth = ctx.data::<Option<AuthHeader>>()?;
        let headers = auth.as_ref().map(AuthHeader::as_header_map);
        let client = ctx.data::<TiledClient>()?;
        let p = self
            .attrs
            .ancestors
            .iter()
            .chain(vec![&self.id])
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("/");
        info!("path: {:?}", p);

        let table_data = client.table_full(&p, columns, headers).await?;
        Ok(table_data)
    }
}

struct Run {
    data: node::Data,
}

#[Object]
impl Run {
    async fn scan_number(&self) -> Option<i64> {
        if let NodeAttributes::Container(attr) = &*self.data.attributes {
            attr.metadata.start_doc().map(|sd| sd.scan_id)
        } else {
            None
        }
    }
    async fn id(&self) -> &str {
        &self.data.id
    }
    async fn data(&self, ctx: &Context<'_>) -> Result<Vec<RunData<'_>>> {
        let auth = ctx.data::<Option<AuthHeader>>()?;
        let headers = auth.as_ref().map(AuthHeader::as_header_map);
        let client = ctx.data::<TiledClient>()?;
        let run_data = client
            .search(
                &self.data.id,
                headers.clone(),
                &[("include_data_sources", "true".into())],
            )
            .await?;
        let mut sources = Vec::new();
        for stream in run_data.data() {
            let stream_data = client
                .search(
                    &format!("{}/{}", self.data.id, stream.id),
                    headers.clone(),
                    &[("include_data_sources", "true".into())],
                )
                .await?;
            for dataset in stream_data.into_data() {
                match *dataset.attributes {
                    NodeAttributes::Array(attrs) => sources.push(RunData::Array(ArrayData {
                        run: self,
                        stream: stream.id.clone(),
                        id: dataset.id,
                        attrs,
                    })),
                    NodeAttributes::Table(attrs) => sources.push(RunData::Internal(TableData {
                        id: dataset.id,
                        attrs,
                    })),
                    NodeAttributes::Container(_) => {}
                }
            }
        }
        Ok(sources)
    }
}

#[cfg(test)]
mod tests {
    use async_graphql::{EmptyMutation, EmptySubscription, Schema, value};
    use axum::http::HeaderValue;
    use httpmock::MockServer;
    use serde_json::json;

    use crate::TiledQuery;
    use crate::clients::TiledClient;
    use crate::handlers::AuthHeader;

    fn build_schema(url: &str) -> Schema<TiledQuery, EmptyMutation, EmptySubscription> {
        Schema::build(TiledQuery, EmptyMutation, EmptySubscription)
            .data(Option::<AuthHeader>::None)
            .data(TiledClient::new(url.parse().unwrap()))
            .finish()
    }

    #[tokio::test]
    async fn app_metadata() {
        let server = MockServer::start();
        let mock = server
            .mock_async(|when, then| {
                when.method("GET").path("/api/v1/");
                then.status(200)
                    .body_from_file("resources/metadata_app.json");
            })
            .await;
        let schema = build_schema(&server.base_url());
        let response = schema.execute("{appMetadata { apiVersion } }").await;

        assert_eq!(response.data, value! {{"appMetadata": {"apiVersion": 0}}});
        assert_eq!(response.errors, &[]);
        mock.assert();
    }

    #[tokio::test]
    async fn invalid_runs() {
        let server = MockServer::start();
        let mock_root = server
            .mock_async(|when, then| {
                when.method("GET").path("/api/v1/search/");
                then.status(200)
                    // File has two run entries where one is not deserializable
                    .body_from_file("resources/search_root_errors.json");
            })
            .await;
        let schema = build_schema(&server.base_url());
        let response = schema
            .execute(
                r#"{instrumentSession(name: "cm12345-2") {
                    runs {
                        id
                    }
                }}"#,
            )
            .await;
        assert_eq!(response.errors, &[]);
        assert_eq!(
            response.data,
            value!({"instrumentSession": {"runs": [{"id": "1e37c0ed-e87e-470d-be18-9d7f62f69127"}]}})
        );
        mock_root.assert_async().await;
    }

    #[tokio::test]
    async fn auth_forwarding() {
        let server = MockServer::start();
        let mock_instrument_session = server
            .mock_async(|when, then| {
                when.method("GET")
                    .path("/api/v1/search/")
                    .query_param("filter[eq][condition][key]", "start.instrument_session")
                    .query_param("filter[eq][condition][value]", r#""cm12345-6""#)
                    .header("Authorization", "auth_value");
                then.status(200).json_body(json!({
                    "data": [],
                    "error": null,
                    "links": {"self":""},
                    "meta": {}
                }));
            })
            .await;
        let schema = Schema::build(TiledQuery, EmptyMutation, EmptySubscription)
            .data(TiledClient::new(server.base_url().parse().unwrap()))
            .data(Some(AuthHeader::from(HeaderValue::from_static(
                "auth_value",
            ))))
            .finish();
        let response = schema
            .execute(r#"{ instrumentSession(name: "cm12345-6"){ runs { id }}}"#)
            .await;
        assert_eq!(response.errors, &[]);
        assert_eq!(response.data, value!({"instrumentSession": {"runs": []}}));
        mock_instrument_session.assert();
    }
}
