use std::error::Error;

pub mod config;
pub mod microsoft_planetary;
pub mod nasa_earthdata;
pub mod s3;
pub mod sattelite_catalog_service;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    println!("🚀 Запуск Satellite Catalog Service конвейера...");

    let mut service = sattelite_catalog_service::SatelliteCatalogService::init().await?;

    service.run_mirroring().await?;

    println!("\n🎉 Зеркалирование всех источников успешно завершено!");

    Ok(())
}
