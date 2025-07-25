//! Contains Configuration of the service Module
#![allow(clippy::ref_option)]

use core::result::Result::Ok;
use std::{
    collections::HashSet,
    convert::Infallible,
    net::{IpAddr, Ipv4Addr},
    ops::{Deref, DerefMut},
    path::PathBuf,
    str::FromStr,
    sync::LazyLock,
    time::Duration,
};

use anyhow::{anyhow, Context};
use http::HeaderValue;
use itertools::Itertools;
use serde::{Deserialize, Deserializer, Serialize};
use url::Url;
use veil::Redact;

use crate::{ProjectId, WarehouseId};

const DEFAULT_RESERVED_NAMESPACES: [&str; 3] = ["system", "examples", "information_schema"];
const DEFAULT_ENCRYPTION_KEY: &str = "<This is unsafe, please set a proper key>";

pub static CONFIG: LazyLock<DynAppConfig> = LazyLock::new(get_config);
pub static DEFAULT_PROJECT_ID: LazyLock<Option<ProjectId>> = LazyLock::new(|| {
    CONFIG
        .enable_default_project
        .then_some(uuid::Uuid::nil().into())
});

fn get_config() -> DynAppConfig {
    let defaults = figment::providers::Serialized::defaults(DynAppConfig::default());

    #[cfg(not(test))]
    let prefixes = &["ICEBERG_REST__", "LAKEKEEPER__"];
    #[cfg(test)]
    let prefixes = &["LAKEKEEPER_TEST__"];

    let file_keys = &["kafka_config"];

    let mut config = figment::Figment::from(defaults);
    for prefix in prefixes {
        let env = figment::providers::Env::prefixed(prefix).split("__");
        config = config
            .merge(figment_file_provider_adapter::FileAdapter::wrap(env.clone()).only(file_keys))
            .merge(env);
    }

    let mut config = config
        .extract::<DynAppConfig>()
        .expect("Valid Configuration");

    // Ensure base_uri has a trailing slash
    if let Some(base_uri) = config.base_uri.as_mut() {
        let base_uri_path = base_uri.path().to_string();
        base_uri.set_path(&format!("{}/", base_uri_path.trim_end_matches('/')));
    }

    config
        .reserved_namespaces
        .extend(DEFAULT_RESERVED_NAMESPACES.into_iter().map(str::to_string));

    // Fail early if the base_uri is not a valid URL
    if let Some(uri) = &config.base_uri {
        uri.join("catalog").expect("Valid URL");
        uri.join("management").expect("Valid URL");
    }

    if config.secret_backend == SecretBackend::Postgres
        && config.pg_encryption_key == DEFAULT_ENCRYPTION_KEY
    {
        tracing::warn!("THIS IS UNSAFE! Using default encryption key for secrets in postgres, please set a proper key using ICEBERG_REST__PG_ENCRYPTION_KEY environment variable.");
    }

    config
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Deserialize, Serialize, Redact)]
/// Configuration of this Module
pub struct DynAppConfig {
    /// Base URL for this REST Catalog.
    /// This is used as the "uri" and "s3.signer.url"
    /// while generating the Catalog Config
    pub base_uri: Option<url::Url>,
    /// Port under which we serve metrics
    pub metrics_port: u16,
    /// Port to listen on.
    pub listen_port: u16,
    /// Bind IP the server listens on.
    /// Defaults to 0.0.0.0
    pub bind_ip: IpAddr,
    /// If x-forwarded-x headers should be respected.
    /// Defaults to true
    pub use_x_forwarded_headers: bool,
    /// If true (default), the NIL uuid is used as default project id.
    pub enable_default_project: bool,
    /// If true, the swagger UI is served at /swagger-ui
    pub serve_swagger_ui: bool,
    /// Template to obtain the "prefix" for a warehouse,
    /// may contain `{warehouse_id}` placeholder.
    ///
    /// If this prefix contains more path segments than the
    /// `warehouse_id`, make sure to strip them using a
    /// reverse proxy before routing to the catalog service.
    /// Example value: `{warehouse_id}`
    prefix_template: String,
    /// CORS allowed origins.
    #[serde(
        deserialize_with = "deserialize_origin",
        serialize_with = "serialize_origin"
    )]
    pub allow_origin: Option<Vec<HeaderValue>>,
    /// Reserved namespaces that cannot be created by users.
    /// This is used to prevent users to create certain
    /// (sub)-namespaces. By default, `system` and `examples` are
    /// reserved. More namespaces can be added here.
    #[serde(
        deserialize_with = "deserialize_reserved_namespaces",
        serialize_with = "serialize_reserved_namespaces"
    )]
    pub reserved_namespaces: ReservedNamespaces,
    // ------------- STORAGE OPTIONS -------------
    /// If true, can create Warehouses with using System Identities.
    pub(crate) enable_aws_system_credentials: bool,
    /// If false, System Identities cannot be used directly to access files.
    /// Instead, `assume_role_arn` must be provided by the user if `SystemIdentities` are used.
    pub(crate) s3_enable_direct_system_credentials: bool,
    /// If true, users must set `external_id` when using system identities with
    /// `assume_role_arn`.
    pub(crate) s3_require_external_id_for_system_credentials: bool,

    /// Enable Azure System Identities
    pub(crate) enable_azure_system_credentials: bool,

    /// Enable GCP System Identities
    pub(crate) enable_gcp_system_credentials: bool,

    // ------------- POSTGRES IMPLEMENTATION -------------
    #[redact]
    pub(crate) pg_encryption_key: String,
    pub(crate) pg_database_url_read: Option<String>,
    pub(crate) pg_database_url_write: Option<String>,
    pub(crate) pg_host_r: Option<String>,
    pub(crate) pg_host_w: Option<String>,
    pub(crate) pg_port: Option<u16>,
    pub(crate) pg_user: Option<String>,
    #[redact]
    pub(crate) pg_password: Option<String>,
    pub(crate) pg_database: Option<String>,
    pub(crate) pg_ssl_mode: Option<PgSslMode>,
    pub(crate) pg_ssl_root_cert: Option<PathBuf>,
    pub(crate) pg_enable_statement_logging: bool,
    pub(crate) pg_test_before_acquire: bool,
    pub(crate) pg_connection_max_lifetime: Option<u64>,
    pub pg_read_pool_connections: u32,
    pub pg_write_pool_connections: u32,
    pub pg_acquire_timeout: u64,

    // ------------- NATS CLOUDEVENTS -------------
    pub nats_address: Option<Url>,
    pub nats_topic: Option<String>,
    pub nats_creds_file: Option<PathBuf>,
    pub nats_user: Option<String>,
    #[redact]
    pub nats_password: Option<String>,
    #[redact]
    pub nats_token: Option<String>,

    // ------------- KAFKA CLOUDEVENTS -------------
    pub kafka_topic: Option<String>,
    #[cfg(feature = "kafka")]
    pub kafka_config: Option<crate::service::event_publisher::kafka::KafkaConfig>,

    // ------------- TRACING CLOUDEVENTS ----------
    pub log_cloudevents: Option<bool>,

    // ------------- AUTHENTICATION -------------
    pub openid_provider_uri: Option<Url>,
    /// Expected audience for the provided token.
    /// Specify multiple audiences as a comma-separated list.
    #[serde(
        deserialize_with = "deserialize_audience",
        serialize_with = "serialize_audience"
    )]
    pub openid_audience: Option<Vec<String>>,
    /// Additional issuers to trust for `OpenID` Connect
    #[serde(
        deserialize_with = "deserialize_audience",
        serialize_with = "serialize_audience"
    )]
    pub openid_additional_issuers: Option<Vec<String>>,
    /// A scope that must be present in provided tokens
    pub openid_scope: Option<String>,
    pub enable_kubernetes_authentication: bool,
    /// Audience expected in provided JWT tokens.
    #[serde(
        deserialize_with = "deserialize_audience",
        serialize_with = "serialize_audience"
    )]
    pub kubernetes_authentication_audience: Option<Vec<String>>,
    /// Accept legacy k8s token without audience and issuer
    /// set to kubernetes/serviceaccount or `https://kubernetes.default.svc.cluster.local`
    pub kubernetes_authentication_accept_legacy_serviceaccount: bool,
    /// Claim to use in provided JWT tokens as the subject.
    pub openid_subject_claim: Option<String>,

    // ------------- AUTHORIZATION - OPENFGA -------------
    #[serde(default)]
    pub authz_backend: AuthZBackend,
    #[serde(
        deserialize_with = "deserialize_openfga_config",
        serialize_with = "serialize_openfga_config"
    )]
    pub openfga: Option<OpenFGAConfig>,

    // ------------- Health -------------
    pub health_check_frequency_seconds: u64,

    // ------------- KV2 -------------
    pub kv2: Option<KV2Config>,
    // ------------- Secrets -------------
    pub secret_backend: SecretBackend,
    #[serde(
        deserialize_with = "crate::config::seconds_to_std_duration",
        serialize_with = "crate::config::serialize_std_duration_as_ms"
    )]
    pub task_poll_interval: std::time::Duration,
    // ------------- Tabular -------------
    /// Delay in seconds after which a tabular will be deleted
    #[serde(
        deserialize_with = "seconds_to_duration",
        serialize_with = "duration_to_seconds"
    )]
    pub default_tabular_expiration_delay_seconds: chrono::Duration,

    // ------------- Page size for paginated queries -------------
    pub pagination_size_default: u32,
    pub pagination_size_max: u32,

    // ------------- Stats -------------
    /// Interval to wait before writing the latest accumulated endpoint statistics into the database.
    ///
    /// Accepts a string of format "{number}{ms|s}", e.g. "30s" for 30 seconds or "500ms" for 500
    /// milliseconds.
    #[serde(
        deserialize_with = "seconds_to_std_duration",
        serialize_with = "serialize_std_duration_as_ms"
    )]
    pub endpoint_stat_flush_interval: Duration,

    // ------------- Internal -------------
    /// Optional server id. We recommend to not change this unless multiple catalogs
    /// are sharing the same Authorization system.
    /// If not specified, 00000000-0000-0000-0000-000000000000 is used.
    /// This ID must not be changed after start!
    #[serde(default = "uuid::Uuid::nil")]
    pub server_id: uuid::Uuid,

    // ------------- Testing -------------
    pub skip_storage_validation: bool,
}

pub(crate) fn seconds_to_duration<'de, D>(deserializer: D) -> Result<chrono::Duration, D::Error>
where
    D: Deserializer<'de>,
{
    let buf = String::deserialize(deserializer)?;

    Ok(chrono::Duration::seconds(
        i64::from_str(&buf).map_err(serde::de::Error::custom)?,
    ))
}

pub(crate) fn duration_to_seconds<S>(
    duration: &chrono::Duration,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    duration.num_seconds().to_string().serialize(serializer)
}

pub(crate) fn seconds_to_std_duration<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    let buf = String::deserialize(deserializer)?;
    Ok(if buf.ends_with("ms") {
        Duration::from_millis(
            u64::from_str(&buf[..buf.len() - 2]).map_err(serde::de::Error::custom)?,
        )
    } else if buf.ends_with('s') {
        Duration::from_secs(u64::from_str(&buf[..buf.len() - 1]).map_err(serde::de::Error::custom)?)
    } else {
        Duration::from_secs(u64::from_str(&buf).map_err(serde::de::Error::custom)?)
    })
}

pub(crate) fn serialize_std_duration_as_ms<S>(
    duration: &Duration,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    format!("{}ms", duration.as_millis()).serialize(serializer)
}

fn deserialize_audience<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    let buf = Option::<serde_json::Value>::deserialize(deserializer)?;
    buf.map(|buf| {
        buf.as_str()
            .map(str::to_string)
            .or(buf.as_i64().map(|i| i.to_string()))
            .map(|s| s.split(',').map(str::to_string).collect::<Vec<_>>())
            .ok_or_else(|| serde::de::Error::custom("Expected a string"))
    })
    .transpose()
}

fn serialize_audience<S>(value: &Option<Vec<String>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    value
        .as_deref()
        .map(|value| value.join(","))
        .serialize(serializer)
}

fn deserialize_origin<'de, D>(deserializer: D) -> Result<Option<Vec<HeaderValue>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::deserialize(deserializer)?
        .map(|buf: String| {
            buf.split(',')
                .map(|s| HeaderValue::from_str(s).map_err(serde::de::Error::custom))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()
}

#[allow(clippy::ref_option)]
fn serialize_origin<S>(value: &Option<Vec<HeaderValue>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    value
        .as_deref()
        .map(|value| {
            value
                .iter()
                .map(|hv| hv.to_str().context("Couldn't serialize cors header"))
                .collect::<anyhow::Result<Vec<_>>>()
                .map(|inner| inner.join(","))
        })
        .transpose()
        .map_err(serde::ser::Error::custom)?
        .serialize(serializer)
}

#[derive(Clone, Serialize, Deserialize, PartialEq, veil::Redact)]
#[serde(rename_all = "snake_case")]
pub enum OpenFGAAuth {
    Anonymous,
    ClientCredentials {
        client_id: String,
        #[redact]
        client_secret: String,
        token_endpoint: Url,
        scope: Option<String>,
    },
    #[redact(all)]
    ApiKey(String),
}

impl Default for OpenFGAAuth {
    fn default() -> Self {
        Self::Anonymous
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct OpenFGAConfig {
    /// GRPC Endpoint Url
    pub endpoint: Url,
    /// Store Name - if not specified, `lakekeeper` is used.
    #[serde(default = "default_openfga_store_name")]
    pub store_name: String,
    /// Authentication configuration
    #[serde(default)]
    pub auth: OpenFGAAuth,
    /// Explicitly set the Authorization model prefix.
    /// Defaults to `collaboration` if not set.
    /// We recommend to use this setting only in combination with
    /// `authorization_model_version`
    #[serde(default = "default_openfga_model_prefix")]
    pub authorization_model_prefix: String,
    /// Version of the model to use. If specified, the specified
    /// model version must already exist.
    /// This can be used to roll-back to previously applied model versions
    /// or to connect to externally managed models.
    /// Migration is disabled if the model version is set.
    /// Version should have the format <major>.<minor>.
    pub authorization_model_version: Option<String>,
    /// The maximum number of checks than can be handled by a batch check
    /// request. This is a [configuration option] of the `OpenFGA` server
    /// with default value 50.
    ///
    /// [configuration option]: https://openfga.dev/docs/getting-started/setup-openfga/configuration#OPENFGA_MAX_CHECKS_PER_BATCH_CHECK
    #[serde(default = "default_openfga_max_batch_check_size")]
    pub max_batch_check_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AuthZBackend {
    #[serde(alias = "allowall", alias = "AllowAll", alias = "ALLOWALL")]
    AllowAll,
    #[cfg(feature = "authz-openfga")]
    #[serde(alias = "openfga", alias = "OpenFGA", alias = "OPENFGA")]
    OpenFGA,
}

impl Default for AuthZBackend {
    fn default() -> Self {
        Self::AllowAll
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SecretBackend {
    #[serde(alias = "kv2", alias = "Kv2")]
    KV2,
    #[serde(alias = "postgres")]
    Postgres,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Redact)]
pub struct KV2Config {
    pub url: Url,
    pub user: String,
    #[redact]
    pub password: String,
    pub secret_mount: String,
}

impl Default for DynAppConfig {
    fn default() -> Self {
        Self {
            base_uri: None,
            metrics_port: 9000,
            enable_default_project: true,
            use_x_forwarded_headers: true,
            prefix_template: "{warehouse_id}".to_string(),
            allow_origin: None,
            reserved_namespaces: ReservedNamespaces(HashSet::from([
                "system".to_string(),
                "examples".to_string(),
            ])),
            pg_encryption_key: DEFAULT_ENCRYPTION_KEY.to_string(),
            pg_database_url_read: None,
            pg_database_url_write: None,
            pg_host_r: None,
            pg_host_w: None,
            pg_port: None,
            pg_user: None,
            pg_password: None,
            pg_database: None,
            pg_ssl_mode: None,
            pg_ssl_root_cert: None,
            pg_enable_statement_logging: false,
            pg_test_before_acquire: false,
            pg_connection_max_lifetime: None,
            pg_read_pool_connections: 10,
            pg_write_pool_connections: 5,
            pg_acquire_timeout: 5,
            enable_azure_system_credentials: false,
            enable_aws_system_credentials: false,
            s3_enable_direct_system_credentials: false,
            s3_require_external_id_for_system_credentials: true,
            enable_gcp_system_credentials: false,
            nats_address: None,
            nats_topic: None,
            nats_creds_file: None,
            nats_user: None,
            nats_password: None,
            nats_token: None,
            #[cfg(feature = "kafka")]
            kafka_config: None,
            kafka_topic: None,
            log_cloudevents: None,
            openid_provider_uri: None,
            openid_audience: None,
            openid_additional_issuers: None,
            openid_scope: None,
            enable_kubernetes_authentication: false,
            kubernetes_authentication_audience: None,
            kubernetes_authentication_accept_legacy_serviceaccount: false,
            openid_subject_claim: None,
            listen_port: 8181,
            bind_ip: IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
            health_check_frequency_seconds: 10,
            kv2: None,
            authz_backend: AuthZBackend::AllowAll,
            openfga: None,
            secret_backend: SecretBackend::Postgres,
            task_poll_interval: Duration::from_secs(10),
            default_tabular_expiration_delay_seconds: chrono::Duration::days(7),
            pagination_size_default: 100,
            pagination_size_max: 1000,
            endpoint_stat_flush_interval: Duration::from_secs(30),
            server_id: uuid::Uuid::nil(),
            serve_swagger_ui: true,
            skip_storage_validation: false,
        }
    }
}

impl DynAppConfig {
    pub fn warehouse_prefix(&self, warehouse_id: WarehouseId) -> String {
        self.prefix_template
            .replace("{warehouse_id}", warehouse_id.to_string().as_str())
    }

    pub fn tabular_expiration_delay(&self) -> chrono::Duration {
        self.default_tabular_expiration_delay_seconds
    }

    pub fn authn_enabled(&self) -> bool {
        self.openid_provider_uri.is_some()
    }

    /// Helper for common conversion of optional page size to `i64`.
    pub fn page_size_or_pagination_max(&self, page_size: Option<i64>) -> i64 {
        page_size.map_or(self.pagination_size_max.into(), |i| {
            i.clamp(1, self.pagination_size_max.into())
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub enum PgSslMode {
    Disable,
    Allow,
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

impl From<PgSslMode> for sqlx::postgres::PgSslMode {
    fn from(value: PgSslMode) -> Self {
        match value {
            PgSslMode::Disable => sqlx::postgres::PgSslMode::Disable,
            PgSslMode::Allow => sqlx::postgres::PgSslMode::Allow,
            PgSslMode::Prefer => sqlx::postgres::PgSslMode::Prefer,
            PgSslMode::Require => sqlx::postgres::PgSslMode::Require,
            PgSslMode::VerifyCa => sqlx::postgres::PgSslMode::VerifyCa,
            PgSslMode::VerifyFull => sqlx::postgres::PgSslMode::VerifyFull,
        }
    }
}

impl FromStr for PgSslMode {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_ref() {
            "disabled" => Ok(Self::Disable),
            "allow" => Ok(Self::Allow),
            "prefer" => Ok(Self::Prefer),
            "require" => Ok(Self::Require),
            "verifyca" => Ok(Self::VerifyCa),
            "verifyfull" => Ok(Self::VerifyFull),
            _ => Err(anyhow!("PgSslMode not supported: '{}'", s)),
        }
    }
}

impl<'de> Deserialize<'de> for PgSslMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        PgSslMode::from_str(&s).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReservedNamespaces(HashSet<String>);
impl Deref for ReservedNamespaces {
    type Target = HashSet<String>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ReservedNamespaces {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl FromStr for ReservedNamespaces {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(ReservedNamespaces(
            s.split(',').map(str::to_string).collect(),
        ))
    }
}

fn deserialize_reserved_namespaces<'de, D>(deserializer: D) -> Result<ReservedNamespaces, D::Error>
where
    D: Deserializer<'de>,
{
    let buf = String::deserialize(deserializer)?;

    ReservedNamespaces::from_str(&buf).map_err(serde::de::Error::custom)
}

fn serialize_reserved_namespaces<S>(
    value: &ReservedNamespaces,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    value.0.iter().join(",").serialize(serializer)
}

#[derive(Serialize, Deserialize, PartialEq, veil::Redact)]
struct OpenFGAConfigSerde {
    /// GRPC Endpoint Url
    endpoint: Url,
    /// Store Name - if not specified, `lakekeeper` is used.
    #[serde(default = "default_openfga_store_name")]
    store_name: String,
    #[serde(default = "default_openfga_model_prefix")]
    authorization_model_prefix: String,
    authorization_model_version: Option<String>,
    /// API-Key. If client-id is specified, this is ignored.
    api_key: Option<String>,
    /// Client id
    client_id: Option<String>,
    #[redact]
    /// Client secret
    client_secret: Option<String>,
    /// Scope for the client credentials
    scope: Option<String>,
    /// Token Endpoint to use when exchanging client credentials for an access token.
    token_endpoint: Option<Url>,
    #[serde(default = "default_openfga_max_batch_check_size")]
    max_batch_check_size: usize,
}

fn default_openfga_store_name() -> String {
    "lakekeeper".to_string()
}

fn default_openfga_model_prefix() -> String {
    "collaboration".to_string()
}

fn default_openfga_max_batch_check_size() -> usize {
    50
}

fn deserialize_openfga_config<'de, D>(deserializer: D) -> Result<Option<OpenFGAConfig>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(OpenFGAConfigSerde {
        client_id,
        client_secret,
        scope,
        token_endpoint,
        api_key,
        endpoint,
        store_name,
        authorization_model_prefix,
        authorization_model_version,
        max_batch_check_size,
    }) = Option::<OpenFGAConfigSerde>::deserialize(deserializer)?
    else {
        return Ok(None);
    };

    let auth = if let Some(client_id) = client_id {
        let client_secret = client_secret.ok_or_else(|| {
            serde::de::Error::custom(
                "openfga client_secret is required when client_id is specified",
            )
        })?;
        let token_endpoint = token_endpoint.ok_or_else(|| {
            serde::de::Error::custom(
                "openfga token_endpoint is required when client_id is specified",
            )
        })?;
        OpenFGAAuth::ClientCredentials {
            client_id,
            client_secret,
            token_endpoint,
            scope,
        }
    } else {
        api_key.map_or(OpenFGAAuth::Anonymous, OpenFGAAuth::ApiKey)
    };

    Ok(Some(OpenFGAConfig {
        endpoint,
        store_name,
        auth,
        authorization_model_prefix,
        authorization_model_version,
        max_batch_check_size,
    }))
}

#[allow(clippy::ref_option)]
fn serialize_openfga_config<S>(
    value: &Option<OpenFGAConfig>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let Some(value) = value else {
        return None::<OpenFGAConfigSerde>.serialize(serializer);
    };

    let (client_id, client_secret, token_endpoint, scope, api_key) = match &value.auth {
        OpenFGAAuth::ClientCredentials {
            client_id,
            client_secret,
            token_endpoint,
            scope,
        } => (
            Some(client_id),
            Some(client_secret),
            Some(token_endpoint),
            scope.clone(),
            None,
        ),
        OpenFGAAuth::ApiKey(api_key) => (None, None, None, None, Some(api_key.clone())),
        OpenFGAAuth::Anonymous => (None, None, None, None, None),
    };

    OpenFGAConfigSerde {
        client_id: client_id.cloned(),
        client_secret: client_secret.cloned(),
        token_endpoint: token_endpoint.cloned(),
        scope,
        api_key,
        endpoint: value.endpoint.clone(),
        store_name: value.store_name.clone(),
        authorization_model_prefix: value.authorization_model_prefix.clone(),
        authorization_model_version: value.authorization_model_version.clone(),
        max_batch_check_size: value.max_batch_check_size,
    }
    .serialize(serializer)
}

#[cfg(test)]
mod test {
    use std::{collections::HashMap, io::Write as _, net::Ipv6Addr};

    #[allow(unused_imports)]
    use super::*;
    #[cfg(feature = "kafka")]
    use crate::service::event_publisher::kafka::KafkaConfig;

    #[test]
    fn test_pg_ssl_mode_case_insensitive() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__PG_SSL_MODE", "DISABLED");
            let config = get_config();
            assert_eq!(config.pg_ssl_mode, Some(PgSslMode::Disable));
            Ok(())
        });
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__PG_SSL_MODE", "DisaBled");
            let config = get_config();
            assert_eq!(config.pg_ssl_mode, Some(PgSslMode::Disable));
            Ok(())
        });
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__PG_SSL_MODE", "disabled");
            let config = get_config();
            assert_eq!(config.pg_ssl_mode, Some(PgSslMode::Disable));
            Ok(())
        });
    }

    #[test]
    fn test_base_uri_trailing_slash_stripped() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__BASE_URI", "https://localhost:8181/a/b/");
            let config = get_config();
            assert_eq!(
                config.base_uri.as_ref().unwrap().to_string(),
                "https://localhost:8181/a/b/"
            );
            assert_eq!(config.base_uri.as_ref().unwrap().path(), "/a/b/");
            Ok(())
        });
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__BASE_URI", "https://localhost:8181/a/b");
            let config = get_config();
            assert_eq!(
                config.base_uri.as_ref().unwrap().to_string(),
                "https://localhost:8181/a/b/"
            );
            assert_eq!(config.base_uri.as_ref().unwrap().path(), "/a/b/");
            Ok(())
        });
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__BASE_URI", "https://localhost:8181");
            let config = get_config();
            assert_eq!(
                config.base_uri.as_ref().unwrap().to_string(),
                "https://localhost:8181/"
            );
            assert_eq!(config.base_uri.as_ref().unwrap().path(), "/");
            Ok(())
        });
    }

    #[test]
    fn test_wildcard_allow_origin() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__ALLOW_ORIGIN", "*");
            let config = get_config();
            assert_eq!(
                config.allow_origin,
                Some(vec![HeaderValue::from_str("*").unwrap()])
            );
            Ok(())
        });
    }

    #[test]
    fn test_single_audience() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__OPENID_AUDIENCE", "abc");
            let config = get_config();
            assert_eq!(config.openid_audience, Some(vec!["abc".to_string()]));
            Ok(())
        });
    }

    #[test]
    fn test_audience_only_numbers() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__OPENID_AUDIENCE", "123456");
            let config = get_config();
            assert_eq!(config.openid_audience, Some(vec!["123456".to_string()]));
            Ok(())
        });
    }

    #[test]
    fn test_multiple_allow_origin() {
        figment::Jail::expect_with(|jail| {
            jail.set_env(
                "LAKEKEEPER_TEST__ALLOW_ORIGIN",
                "http://localhost,http://example.com",
            );
            let config = get_config();
            assert_eq!(
                config.allow_origin,
                Some(vec![
                    HeaderValue::from_str("http://localhost").unwrap(),
                    HeaderValue::from_str("http://example.com").unwrap()
                ])
            );
            Ok(())
        });
    }

    #[test]
    fn test_default() {
        let _ = &CONFIG.base_uri;
    }

    #[test]
    fn test_queue_config() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__TASK_POLL_INTERVAL", "5s");
            let config = get_config();
            assert_eq!(config.task_poll_interval, Duration::from_secs(5));
            Ok(())
        });
    }

    #[test]
    fn reserved_namespaces_should_contains_default_values() {
        assert!(CONFIG.reserved_namespaces.contains("system"));
        assert!(CONFIG.reserved_namespaces.contains("examples"));
    }

    #[test]
    fn test_task_queue_config_millis() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__TASK_POLL_INTERVAL", "5ms");
            let config = get_config();
            assert_eq!(
                config.task_poll_interval,
                std::time::Duration::from_millis(5)
            );
            Ok(())
        });
    }

    #[test]
    fn test_task_queue_config_seconds() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__TASK_POLL_INTERVAL", "5s");
            let config = get_config();
            assert_eq!(config.task_poll_interval, std::time::Duration::from_secs(5));
            Ok(())
        });
    }

    #[test]
    fn test_task_queue_config_legacy_seconds() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__TASK_POLL_INTERVAL", "\"5\"");
            let config = get_config();
            assert_eq!(config.task_poll_interval, std::time::Duration::from_secs(5));
            Ok(())
        });
    }

    #[cfg(feature = "authz-openfga")]
    #[test]
    fn test_openfga_config_no_auth() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__AUTHZ_BACKEND", "openfga");
            jail.set_env("LAKEKEEPER_TEST__OPENFGA__ENDPOINT", "http://localhost");
            jail.set_env("LAKEKEEPER_TEST__OPENFGA__STORE_NAME", "store_name");
            let config = get_config();
            let authz_config = config.openfga.unwrap();
            assert_eq!(config.authz_backend, AuthZBackend::OpenFGA);
            assert_eq!(authz_config.store_name, "store_name");

            assert_eq!(authz_config.auth, OpenFGAAuth::Anonymous);

            Ok(())
        });
    }

    #[cfg(feature = "authz-openfga")]
    #[test]
    fn test_openfga_config_api_key() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__AUTHZ_BACKEND", "openfga");
            jail.set_env("LAKEKEEPER_TEST__OPENFGA__ENDPOINT", "http://localhost");
            jail.set_env("LAKEKEEPER_TEST__OPENFGA__API_KEY", "api_key");
            let config = get_config();
            let authz_config = config.openfga.unwrap();
            assert_eq!(config.authz_backend, AuthZBackend::OpenFGA);
            assert_eq!(authz_config.store_name, "lakekeeper");

            assert_eq!(
                authz_config.auth,
                OpenFGAAuth::ApiKey("api_key".to_string())
            );
            Ok(())
        });
    }

    #[test]
    #[should_panic(expected = "openfga client_secret is required when client_id is specified")]
    fn test_openfga_client_config_fails_without_token() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__AUTHZ_BACKEND", "openfga");
            jail.set_env("LAKEKEEPER_TEST__OPENFGA__ENDPOINT", "http://localhost");
            jail.set_env("LAKEKEEPER_TEST__OPENFGA__CLIENT_ID", "client_id");
            jail.set_env("LAKEKEEPER_TEST__OPENFGA__STORE_NAME", "store_name");
            get_config();
            Ok(())
        });
    }

    #[cfg(feature = "authz-openfga")]
    #[test]
    fn test_openfga_client_credentials() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__AUTHZ_BACKEND", "openfga");
            jail.set_env("LAKEKEEPER_TEST__OPENFGA__ENDPOINT", "http://localhost");
            jail.set_env("LAKEKEEPER_TEST__OPENFGA__CLIENT_ID", "client_id");
            jail.set_env("LAKEKEEPER_TEST__OPENFGA__CLIENT_SECRET", "client_secret");
            jail.set_env(
                "LAKEKEEPER_TEST__OPENFGA__TOKEN_ENDPOINT",
                "https://example.com/token",
            );
            let config = get_config();
            let authz_config = config.openfga.unwrap();
            assert_eq!(config.authz_backend, AuthZBackend::OpenFGA);
            assert_eq!(authz_config.store_name, "lakekeeper");

            assert_eq!(
                authz_config.auth,
                OpenFGAAuth::ClientCredentials {
                    client_id: "client_id".to_string(),
                    client_secret: "client_secret".to_string(),
                    token_endpoint: "https://example.com/token".parse().unwrap(),
                    scope: None
                }
            );
            Ok(())
        });
    }

    #[test]
    fn test_openfga_client_credentials_with_scope() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__AUTHZ_BACKEND", "openfga");
            jail.set_env("LAKEKEEPER_TEST__OPENFGA__ENDPOINT", "http://localhost");
            jail.set_env("LAKEKEEPER_TEST__OPENFGA__CLIENT_ID", "client_id");
            jail.set_env("LAKEKEEPER_TEST__OPENFGA__CLIENT_SECRET", "client_secret");
            jail.set_env("LAKEKEEPER_TEST__OPENFGA__SCOPE", "openfga");
            jail.set_env(
                "LAKEKEEPER_TEST__OPENFGA__TOKEN_ENDPOINT",
                "https://example.com/token",
            );
            let config = get_config();
            let authz_config = config.openfga.unwrap();

            assert_eq!(
                authz_config.auth,
                OpenFGAAuth::ClientCredentials {
                    client_id: "client_id".to_string(),
                    client_secret: "client_secret".to_string(),
                    token_endpoint: "https://example.com/token".parse().unwrap(),
                    scope: Some("openfga".to_string())
                }
            );
            Ok(())
        });
    }

    #[test]
    fn test_bind_ip_address_v4_all() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__BIND_IP", "0.0.0.0");
            let config = get_config();
            assert_eq!(config.bind_ip, IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)));
            Ok(())
        });
    }

    #[test]
    fn test_bind_ip_address_v4_localhost() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__BIND_IP", "127.0.0.1");
            let config = get_config();
            assert_eq!(config.bind_ip, IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));
            Ok(())
        });
    }

    #[test]
    fn test_bind_ip_address_v6_loopback() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__BIND_IP", "::1");
            let config = get_config();
            assert_eq!(
                config.bind_ip,
                IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1))
            );
            Ok(())
        });
    }

    #[test]
    fn test_bind_ip_address_v6_all() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__BIND_IP", "::");
            let config = get_config();
            assert_eq!(
                config.bind_ip,
                IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0))
            );
            Ok(())
        });
    }

    #[test]
    fn test_legacy_service_account_acceptance() {
        figment::Jail::expect_with(|jail| {
            jail.set_env(
                "LAKEKEEPER_TEST__KUBERNETES_AUTHENTICATION_ACCEPT_LEGACY_SERVICEACCOUNT",
                "true",
            );
            let config = get_config();
            assert!(config.kubernetes_authentication_accept_legacy_serviceaccount);
            Ok(())
        });
    }

    #[test]
    fn test_s3_disable_system_credentials() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__ENABLE_AWS_SYSTEM_CREDENTIALS", "true");
            let config = get_config();
            assert!(config.enable_aws_system_credentials);
            assert!(!config.s3_enable_direct_system_credentials);
            Ok(())
        });
    }

    #[test]
    fn test_use_x_forwarded_headers() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__USE_X_FORWARDED_HEADERS", "true");
            let config = get_config();
            assert!(config.use_x_forwarded_headers);
            Ok(())
        });

        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__USE_X_FORWARDED_HEADERS", "false");
            let config = get_config();
            assert!(!config.use_x_forwarded_headers);
            Ok(())
        });
    }

    #[cfg(feature = "kafka")]
    #[test]
    fn test_kafka_config_env_var() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__KAFKA_TOPIC", "test_topic");
            jail.set_env(
                "LAKEKEEPER_TEST__KAFKA_CONFIG",
                r#"{"sasl.password"="my_pw","bootstrap.servers"="host1:port,host2:port","security.protocol"="SSL"}"#,
            );
            jail.set_env(
                "LAKEKEEPER_TEST__KAFKA_CONFIG_FILE",
                r#"{"sasl.password"="my_pw","bootstrap.servers"="host1:port,host2:port","security.protocol"="SSL"}"#,
            );
            let config = get_config();
            assert_eq!(config.kafka_topic, Some("test_topic".to_string()));
            assert_eq!(
                config.kafka_config,
                Some(KafkaConfig {
                    sasl_password: Some("my_pw".to_string()),
                    sasl_oauthbearer_client_secret: None,
                    ssl_key_password: None,
                    ssl_keystore_password: None,
                    conf: HashMap::from_iter([
                        (
                            "bootstrap.servers".to_string(),
                            "host1:port,host2:port".to_string()
                        ),
                        ("security.protocol".to_string(), "SSL".to_string()),
                    ]),
                })
            );
            Ok(())
        });
    }

    #[cfg(feature = "kafka")]
    #[test]
    fn test_kafka_config_file() {
        let named_tmp_file = tempfile::NamedTempFile::new().unwrap();
        named_tmp_file
            .as_file()
            .write_all(
                r#"{"sasl.password"="my_pw","bootstrap.servers"="host1:port,host2:port","security.protocol"="SSL"}"#.as_bytes(),
            )
            .unwrap();
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__KAFKA_TOPIC", "test_topic");
            jail.set_env(
                "LAKEKEEPER_TEST__KAFKA_CONFIG_FILE",
                named_tmp_file.path().to_str().unwrap(),
            );
            let config = get_config();
            assert_eq!(config.kafka_topic, Some("test_topic".to_string()));
            assert_eq!(
                config.kafka_config,
                Some(KafkaConfig {
                    sasl_password: Some("my_pw".to_string()),
                    sasl_oauthbearer_client_secret: None,
                    ssl_key_password: None,
                    ssl_keystore_password: None,
                    conf: HashMap::from_iter([
                        (
                            "bootstrap.servers".to_string(),
                            "host1:port,host2:port".to_string()
                        ),
                        ("security.protocol".to_string(), "SSL".to_string()),
                    ]),
                })
            );
            Ok(())
        });
    }

    #[test]
    fn test_disable_storage_validation() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__SKIP_STORAGE_VALIDATION", "true");
            let config = get_config();
            assert!(config.skip_storage_validation);
            Ok(())
        });

        figment::Jail::expect_with(|jail| {
            jail.set_env("LAKEKEEPER_TEST__SKIP_STORAGE_VALIDATION", "false");
            let config = get_config();
            assert!(!config.skip_storage_validation);
            Ok(())
        });
    }
}
