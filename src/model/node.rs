use std::collections::HashMap;

use async_graphql::{Enum, SimpleObject};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{array, container, table};

pub type Root = Response<Vec<DataOption>>;
pub type Metadata = Response<Data>;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct Response<D> {
    data: D,
    pub error: Value,
    pub links: Option<Links>,
    pub meta: Value,
}

impl Root {
    pub fn data(&self) -> impl Iterator<Item = &Data> {
        self.data.iter().flat_map(DataOption::as_data)
    }
    pub fn into_data(self) -> impl Iterator<Item = Data> {
        self.data.into_iter().flat_map(DataOption::into_data)
    }
}

impl Metadata {
    pub fn into_data(self) -> Data {
        self.data
    }
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DataOption {
    Data(Data),
    Error(Value),
}

impl DataOption {
    pub fn as_data(&self) -> Option<&Data> {
        match self {
            Self::Data(data) => Some(data),
            Self::Error(_) => None,
        }
    }
    pub fn into_data(self) -> Option<Data> {
        match self {
            Self::Data(data) => Some(data),
            Self::Error(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Data {
    pub id: String,
    pub attributes: Box<NodeAttributes>,
    pub links: Box<Links>,
    pub meta: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "structure_family", rename_all = "lowercase")]
pub enum NodeAttributes {
    Container(Attributes<container::ContainerMetadata, container::ContainerStructure>),
    Array(Attributes<HashMap<String, Value>, array::ArrayStructure>),
    Table(Attributes<HashMap<String, Value>, table::TableStructure>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attributes<Meta, S> {
    pub ancestors: Vec<String>,
    pub specs: Vec<Spec>,
    pub metadata: Meta,
    pub structure: S,
    pub access_blob: Value,
    pub sorting: Option<Vec<Sorting>>,
    pub data_sources: Option<Vec<DataSource<S>>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Spec {
    pub name: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sorting {
    pub key: String,
    pub direction: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataSource<S> {
    pub structure: S,
    pub id: Option<u64>,
    pub mimetype: Option<String>,
    pub parameters: HashMap<String, Value>,
    pub assets: Vec<Asset>,
    management: Management,
}

#[derive(Enum, Debug, Copy, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Management {
    External,
    Immutable,
    Locked,
    Writable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, SimpleObject)]
pub struct Asset {
    pub data_uri: String,
    is_directory: bool,
    parameter: Option<String>,
    num: Option<i64>,
    pub id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, SimpleObject)]
pub struct Links {
    #[serde(rename = "self")]
    #[graphql(name = "self")]
    pub self_field: String,
    pub documentation: Option<String>,
    pub first: Option<String>,
    pub last: Option<String>,
    pub next: Option<String>,
    pub prev: Option<String>,
    pub search: Option<String>,
    pub full: Option<String>,
    pub block: Option<String>,
    pub partition: Option<String>,
}
