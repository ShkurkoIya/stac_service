use serde::Deserialize;
use std::fs;

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub infrastructure: InfraConfig,
    pub providers: Vec<ProviderTarget>,
    pub sync_settings: SyncSettings,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SyncSettings {
    pub default_max_cloud_cover: f64,
    pub start_date: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ProviderTarget {
    pub name: String,
    pub api_url: String,
    pub collections: Vec<CollectionTarget>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct InfraConfig {
    pub s3: S3Config,
    pub database: DbConfig,
    pub local_stac: LocalStacConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct S3Config {
    pub endpoint: String,
    pub bucket: Option<String>,
    pub access_key: String,
    pub secret_key: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DbConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub dbname: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LocalStacConfig {
    pub api_url: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CollectionTarget {
    pub id: String,
    pub description: String,
    pub max_cloud_cover: Option<f64>,
    pub bbox: Vec<f64>,
    pub start_date: Option<String>,
}

pub fn load_config() -> Result<AppConfig, Box<dyn std::error::Error + Send + Sync>> {
    let config_content = fs::read_to_string("config.toml")
        .expect("Не удалось найти файл config.toml в корне проекта!");
    let config: AppConfig = toml::from_str(&config_content)?;

    Ok(config)
}
