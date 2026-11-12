use aws_sdk_s3::Client as S3Client;

pub async fn ensure_bucket_exists(
    s3_client: &S3Client,
    bucket_name: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match s3_client.head_bucket().bucket(bucket_name).send().await {
        Ok(_) => {
            println!(
                "[MinIO] Бакет '{}' подтвержден и готов к работе.",
                bucket_name
            );
        }
        Err(_) => {
            println!(
                "[MinIO] Бакет '{}' не найден. Авто-создание...",
                bucket_name
            );
            s3_client.create_bucket().bucket(bucket_name).send().await?;
            println!("🟢 [MinIO] Бакет '{}' успешно создан!", bucket_name);
        }
    }
    Ok(())
}
