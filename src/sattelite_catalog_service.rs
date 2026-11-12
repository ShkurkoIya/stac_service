use crate::config::{AppConfig, CollectionTarget, ProviderTarget, load_config};
use crate::microsoft_planetary::auth::MicrosoftSasMiddleware;
use aws_config::{Region, meta::region::RegionProviderChain};
use aws_sdk_s3::{Client as S3Client, config::Credentials, primitives::ByteStream};
use pgstac::Pgstac;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use serde_json::json;
use stac::Collection;
use stac_client::{Client as StacApiClient, SearchBuilder, SortDirection};
use std::{collections::HashMap, error::Error, path::Path};
use tokio_postgres::{Client as PgClient, NoTls};
use url::Url;

pub type BoxError = Box<dyn Error + Send + Sync + Send>;

pub struct S3Context {
    pub client: S3Client,
    pub is_bucket_auto_naming_mode: bool,
    pub global_bucket_name: Option<String>,
}

pub struct SatelliteCatalogService {
    pub config: AppConfig,
    pub stac_clients: HashMap<String, StacApiClient>,
    pub s3_context: S3Context,
    pub pg_client: PgClient,
    pub http_client: ClientWithMiddleware,
}

impl SatelliteCatalogService {
    pub async fn s3_init(config: &AppConfig) -> Result<S3Context, BoxError> {
        // 1. Настраиваем креды и регион
        let credentials = Credentials::new(
            &config.infrastructure.s3.access_key,
            &config.infrastructure.s3.secret_key,
            None,
            None,
            "StaticConfiguration",
        );

        let region_provider = RegionProviderChain::default_provider().or_else("us-east-1");
        let region = region_provider
            .region()
            .await
            .unwrap_or_else(|| Region::new("us-east-1"));

        // Используем актуальный defaults() вместо устаревшего from_env()
        let shared_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(region)
            .load()
            .await;

        let s3_config = aws_sdk_s3::config::Builder::from(&shared_config)
            .credentials_provider(credentials)
            .endpoint_url(&config.infrastructure.s3.endpoint)
            .force_path_style(true)
            .build();

        let s3_client = S3Client::from_conf(s3_config);

        let bucket_mode: &str = match &config.infrastructure.s3.bucket.as_deref() {
            Some(b) if b.trim().is_empty() => {
                eprintln!("Критическая ошибка: Бро, где бакет? Поле bucket в конфиге пустое!");
                return Err("Missing bucket configuration".into());
            }
            None => {
                eprintln!(
                    "Критическая ошибка: Бро, где бакет? Забыл указать [infrastructure.s3].bucket!"
                );
                return Err("Missing bucket configuration".into());
            }
            Some(b) => b,
        };

        let is_auto_mode = bucket_mode == "auto";
        let mut global_bucket_name: Option<String> = None;
        if !is_auto_mode {
            crate::s3::ensure_bucket_exists(&s3_client, bucket_mode).await?;
            global_bucket_name = Some(bucket_mode.to_string());
        } else {
            println!(
                "[MinIO] Включен режим 'auto'. Бакеты будут генерироваться динамически под каждую коллекцию."
            );
        }

        Ok(S3Context {
            client: s3_client,
            is_bucket_auto_naming_mode: is_auto_mode,
            global_bucket_name,
        })
    }

    async fn init_postgres(config: &AppConfig) -> Result<PgClient, Box<dyn Error + Send + Sync>> {
        let postgres_url = format!(
            "host={} port={} user={} password={} dbname={}",
            config.infrastructure.database.host,
            config.infrastructure.database.port,
            config.infrastructure.database.user,
            config.infrastructure.database.password,
            config.infrastructure.database.dbname
        );

        let (pg_client, connection) = tokio_postgres::connect(&postgres_url, NoTls).await?;

        // Спавним коннект в бэкграунд
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                eprintln!("[Postgres] Ошибка соединения: {}", e);
            }
        });

        // Сразу выставляем search_path, чтобы pgstac не падал
        pg_client
            .execute("SET search_path TO pgstac, public;", &[])
            .await?;
        println!("[Postgres] Успешное подключение, search_path настроен.");

        Ok(pg_client)
    }

    fn init_stac_clients(
        providers: &[ProviderTarget],
    ) -> Result<HashMap<String, StacApiClient>, Box<dyn Error + Send + Sync>> {
        let mut stac_clients = HashMap::new();

        for provider in providers {
            println!(
                "[STAC API] Инициализация клиента для [{}] ({})...",
                provider.name, provider.api_url
            );

            match StacApiClient::new(&provider.api_url) {
                Ok(client) => {
                    stac_clients.insert(provider.name.clone(), client);
                }
                Err(e) => {
                    return Err(format!(
                        "Не удалось создать STAC клиент для провайдера {}: {}",
                        provider.name, e
                    )
                    .into());
                }
            }
        }

        Ok(stac_clients)
    }

    pub async fn init() -> Result<Self, Box<dyn Error + Send + Sync>> {
        let config = load_config()?;

        let raw_client = reqwest::Client::new();
        let http_client = ClientBuilder::new(raw_client)
            .with(MicrosoftSasMiddleware::default())
            .build();

        let s3_context = Self::s3_init(&config).await?;
        let pg_client = Self::init_postgres(&config).await?;
        let stac_clients = Self::init_stac_clients(&config.providers)?;

        Ok(Self {
            config,
            s3_context,
            stac_clients,
            pg_client,
            http_client,
        })
    }

    pub async fn run_mirroring(&mut self) -> Result<(), BoxError> {
        // Чтобы обойти ограничения заимствования (Borrow Checker) при вызове &mut self.mirror_collection,
        // мы клонируем список провайдеров из конфига для итерации. Это дешево и абсолютно безопасно.
        let providers = self.config.providers.clone();

        for provider in &providers {
            println!("\n ========================================================");
            println!(" Работаем с провайдером [{}]", provider.name);
            println!("========================================================");

            // Проверяем наличие клиента в пуле перед стартом
            if !self.stac_clients.contains_key(&provider.name) {
                eprintln!(
                    "⚠️ Предупреждение: Клиент для провайдера {} отсутствует в пуле. Пропускаем.",
                    provider.name
                );
                continue;
            }

            for collection in &provider.collections {
                println!("\n Начинаем обработку целевой коллекции: {}", collection.id);

                // 1. Синхронизируем саму структуру коллекции в pgstac, если её там нет
                // Передаем id коллекции (библиотека pgstac ожидает структуру или её описание)
                if let Err(e) = self.ensure_collection(collection).await {
                    eprintln!(
                        "🔴 Сбой при синхронизации структуры коллекции {}: {}",
                        collection.id, e
                    );
                    continue;
                }

                // 2. Запускаем наш основной конвейер выкачки ассетов и апсерта айтемов
                // Передаем ссылки на конфигурацию текущего провайдера и коллекции
                if let Err(e) = self.mirror_collection(provider, collection).await {
                    eprintln!(
                        "🔴 Сбой при зеркалировании айтемов коллекции {}: {}",
                        collection.id, e
                    );
                }
            }
        }

        Ok(())
    }

    async fn ensure_collection(&mut self, target: &CollectionTarget) -> Result<(), BoxError> {
        let target_collection = Collection::new(&target.id, &target.description);
        let transaction = self.pg_client.transaction().await?;
        let _ = transaction.add_collection(target_collection).await;
        transaction.commit().await?;
        println!(
            "[pgSTAC] Синхронизирована структура коллекции '{}'",
            target.id
        );

        Ok(())
    }

    pub async fn mirror_collection(
        &mut self,
        provider: &ProviderTarget,
        collection: &CollectionTarget,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // 1. Достаем клиент из хэшмапы
        let api_client = self
            .stac_clients
            .get(&provider.name)
            .ok_or_else(|| format!("STAC клиент для провайдера '{}' не найден", provider.name))?;

        // 2. Вычисляем динамические настройки фильтрации (Коллекция -> Глобальный дефолт)
        let max_cloud = collection
            .max_cloud_cover
            .unwrap_or(self.config.sync_settings.default_max_cloud_cover);

        let start_date = collection
            .start_date
            .as_deref()
            .unwrap_or(&self.config.sync_settings.start_date);

        let datetime_filter = format!("{}/..", start_date);

        // 3. Строим поисковый запрос к внешнему API
        let search_query = SearchBuilder::new()
            .limit(100) // Качаем пачками по 3 айтема
            .bbox(collection.bbox.clone())
            .datetime(&datetime_filter)
            .collections(vec![collection.id.clone()])
            .query("eo:cloud_cover", json!({ "lt": max_cloud }))
            .sort_by("datetime", SortDirection::Desc)
            .build();

        let mut result_page = api_client.search(&search_query).await?;

        // Достаем имя нашего единого бакета из S3-контекста
        let bucket_name = self
            .s3_context
            .global_bucket_name
            .clone()
            .expect("Критическая ошибка: Имя глобального бакета в S3Context оказалось пустым!");

        let mut empty_pages_strike = 0;

        loop {
            // Забираем вектор айтемов (features) во владение
            let items = std::mem::take(&mut result_page.features);
            if items.is_empty() {
                println!(
                    "🏁 Потоковые данные для коллекции '{}' исчерпаны.",
                    collection.id
                );
                break;
            }

            println!(
                "Найдено {} айтемов в пачке. Запуск параллельного транзита...",
                items.len()
            );
            let mut items_to_save_in_db = Vec::new();

            for mut item in items {
                println!("  🔹 Обработка айтема: {}", item.id);

                let item_id = item.id.clone();
                let mut need_download = true;

                // Проверяем, какой маркерный ключ превьюшки есть у айтема
                let preview_key = if item.assets.contains_key("thumbnail") {
                    Some("thumbnail")
                } else if item.assets.contains_key("preview") {
                    Some("preview")
                } else {
                    // Если превью нет, берем первый попавшийся ключ ассета как маркер
                    item.assets.keys().next().map(|s| s.as_str())
                };

                let item_exists = self
                    .pg_client
                    .query_opt("SELECT id FROM pgstac.items WHERE id = $1", &[&item_id])
                    .await?
                    .is_some();

                // Проверяем уровень 1: Есть ли метаданные в pgstac?
                if item_exists {
                    if let Some(key) = preview_key {
                        if let Some(asset) = item.assets.get(key) {
                            let extension = Path::new(&asset.href)
                                .extension()
                                .and_then(|ext| ext.to_str())
                                .unwrap_or("tif")
                                .to_lowercase();

                            let filename = Url::parse(&asset.href)
                                .ok()
                                .and_then(|url| {
                                    url.path_segments()?.next_back()?.to_string().into()
                                })
                                .unwrap_or_else(|| format!("{}_{}.{}", item_id, key, extension));

                            let s3_key =
                                format!("{}/{}/{}", provider.name, collection.id, filename);

                            // Проверяем уровень 2: Реально ли физический файл лежит в MinIO?
                            let file_on_s3_exists = self
                                .s3_context
                                .client
                                .head_object()
                                .bucket(&bucket_name)
                                .key(&s3_key)
                                .send()
                                .await
                                .is_ok();

                            if file_on_s3_exists {
                                // ПОЛНЫЙ СИНХРОН: Всё есть и в базе, и на диске! Скипаем весь айтем.
                                println!(
                                    "⏭️ [СИНХРОН] Айтем '{}' полностью засинхронен (pgstac + S3). Пропускаем.",
                                    item_id
                                );
                                need_download = false;
                            } else {
                                // РАССИНХРОН: База помнит, а файлы на S3 пустые! Запускаем лечение
                                println!(
                                    "⚠️  [РАССИНХРОН] '{}' найден в pgstac, но на S3 пусто! Восстанавливаем файлы...",
                                    item_id
                                );
                                need_download = true;
                            }
                        }
                    }
                }

                if !need_download {
                    // Переходим к следующему айтему, полностью пропуская скачивание файлов
                    continue;
                }
                // --- КОНЕЦ БЛОКА ДВУСТОРОННЕЙ ПРОВЕРКИ ---

                let mut assets_to_upload = Vec::new();

                // Формируем список задач на загрузку для текущего айтема
                for (asset_key, asset) in item.assets.iter() {
                    let remote_href = asset.href.clone();
                    let extension = Path::new(&remote_href)
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .unwrap_or("tif")
                        .to_lowercase();

                    let filename = Url::parse(&remote_href)
                        .ok()
                        .and_then(|url| url.path_segments()?.next_back()?.to_string().into())
                        .unwrap_or_else(|| format!("{}_{}.{}", item.id, asset_key, extension));

                    // Виртуальный путь в MinIO: "провайдер/коллекция/файл"
                    let s3_key = format!("{}/{}/{}", provider.name, collection.id, filename);
                    assets_to_upload.push((remote_href, s3_key));
                }

                // Клонируем клиенты для асинхронного Tokio-стрима
                let s3_client = self.s3_context.client.clone();
                let http_client = self.http_client.clone();
                let bucket = bucket_name.clone(); // Чистая String, которая точно существует

                let download_futures = assets_to_upload.into_iter().map(|(remote_href, s3_key)| {
                    let s3 = s3_client.clone();
                    let http = http_client.clone();
                    let b = bucket.clone();

                    async move {
                        // Проверяем, не скачали ли мы файл ранее
                        let file_exists = s3
                            .head_object()
                            .bucket(&b)
                            .key(&s3_key)
                            .send()
                            .await
                            .is_ok();
                        if file_exists {
                            println!("⏭️ [MinIO] Файл уже скачан, пропускаем: {}", s3_key);
                            return;
                        }

                        // Скачиваем ассет
                        let Ok(response) = http.get(&remote_href).send().await else {
                            return;
                        };

                        if !response.status().is_success() {
                            eprintln!("🔴 [HTTP {}] Сбой скачивания {}", response.status(), s3_key);
                            return;
                        }

                        let Ok(bytes) = response.bytes().await else {
                            return;
                        };
                        let body = ByteStream::from(bytes);

                        // Заливаем в MinIO
                        if let Err(e) = s3
                            .put_object()
                            .bucket(&b)
                            .key(&s3_key)
                            .body(body)
                            .send()
                            .await
                        {
                            eprintln!("🔴 [MinIO Ошибка] Сбой аплоада {}: {:?}", s3_key, e);
                        } else {
                            println!("🟢 [MinIO] Успешно сохранен: {}", s3_key);
                        }
                    }
                });

                // Ждем скачивания всех ассетов текущего айтема
                futures::future::join_all(download_futures).await;

                // Переписываем ссылки на ассеты в формат s3:// для pgstac
                for (asset_key, asset) in item.assets.iter_mut() {
                    let extension = Path::new(&asset.href)
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("tif")
                        .to_lowercase();

                    let filename = Url::parse(&asset.href)
                        .ok()
                        .and_then(|url| url.path_segments()?.next_back()?.to_string().into())
                        .unwrap_or_else(|| format!("{}_{}.{}", item.id, asset_key, extension));
                    let s3_key = format!("{}/{}/{}", provider.name, collection.id, filename);
                    asset.href = format!("s3://{}/{}", bucket_name, s3_key);
                }

                // ВАЖНО: Если мы запустили самолечение, то в pgstac этот айтем УЖЕ есть.
                // Чтобы транзакция не упала из-за попытки вставить дубликат, мы сохраняем
                // метаданные в базу ТОЛЬКО если айтема там реально не было.
                if !item_exists {
                    items_to_save_in_db.push(item);
                } else {
                    println!(
                        "🔧 [Самолечение] Файлы для '{}' восстановлены на S3. База в порядке.",
                        item_id
                    );
                }
            }

            // 4. Заливаем пачку измененных метаданных айтемов в PgSTAC в одной транзакции
            if !items_to_save_in_db.is_empty() {
                let tx = self.pg_client.transaction().await?;
                tx.upsert_items(&items_to_save_in_db).await?;
                tx.commit().await?;
                println!(
                    "💾 Метаданные {} айтемов успешно прописаны в pgstac.",
                    items_to_save_in_db.len()
                );

                empty_pages_strike = 0;
            } else {
                println!("⏭️ В текущей пачке нет новых метаданных для записи в pgstac.");
                // Инкрементируем счетчик холостых прогонов
                empty_pages_strike += 1;
            }

            // 🛑 СТОП-КРАН ПРОТИВ ЗАЦИКЛИВАНИЯ API MICROSOFT:
            // Если 3 страницы подряд провайдер сует нам старый мусор — разрываем цикл,
            // значит мы выкачали абсолютно всё целевое окно!
            if empty_pages_strike >= 3 {
                println!(
                    "🛑 [Защита] Обнаружен пагинационный тупик внешнего API. Пайплайн принудительно остановлен."
                );
                break;
            }

            // 5. Пагинация (Запрашиваем следующую страницу у внешнего API)
            let next_link = result_page
                .links
                .as_ref()
                .and_then(|links| links.iter().find(|link| link.rel == "next"));

            if let Some(link) = next_link {
                println!("➡️ Переходим к следующей странице: {}", link.href);
                let response = self.http_client.get(&link.href).send().await?;
                // Перезаписываем ItemCollection обратно в переменную result_page
                // result_page = response.json::<stac_client::ItemCollection>().await?;
                if let Ok(next_page) = response.json::<stac_client::ItemCollection>().await {
                    result_page = next_page;
                } else {
                    println!(
                        "🏁 API вернул невалидный JSON на следующей странице. Считаем, что данные исчерпаны."
                    );
                    break;
                }
            } else {
                println!("🏁 Синхронизация коллекции '{}' завершена.", collection.id);
                break;
            }
        }
        Ok(())
    }
}
