# 🛰️ Satellite Catalog Service (STAC Mirror & Ingestion Engine)

Высокопроизводительный асинхронный микросервис и готовая локальная инфраструктура на языке **Rust** для зеркалирования, индексации и раздачи пространственно-временных каталогов спутниковых данных (**STAC API**).

Сервис реализует сквозной транзитный конвейер без сохранения промежуточных файлов на локальный диск: он подключается к публичным STAC-провайдерам (Microsoft Planetary Computer, AWS Element84, USGS), прозрачно подписывает закрытые запросы через HTTP Middleware, параллельно стримит геоданные (COG, NetCDF, Parquet) напрямую в объектное хранилище **MinIO (S3)**, переписывает ссылки на ассеты с поддержкой расширения `alternate` и выполняет запись метаданных в пространственную СУБД **pgSTAC (PostgreSQL + PostGIS)**.

---

## 🏗 Архитектура экосистемы

```text
[Удаленные STAC APIs] (Microsoft PC, Element84, USGS)
         │
         ▼ (Поиск и фильтрация по BBOX / датам / облачности)
[SatelliteCatalogService] (Rust + Tokio + Reqwest Middleware)
         │
         ├──► [MicrosoftSasMiddleware] (Авто-получение и кэш SAS-токенов Azure)
         │
         ├──► Zero-Disk Стрим (.tif, .nc) ──► [MinIO Object Storage (S3)]
         │                                               │
         ├──► Двусторонний Self-Healing (DB <-> S3) ◄────┘
         │
         ▼ (Upsert метаданных + s3:// / alternate http://)
[Локальная БД pgSTAC] (PostgreSQL + PostGIS)
         │
         ├───────────────────────────┐
         ▼                           ▼
[stac-fastapi-pgstac]       [TiTiler-pgstac]
(REST STAC API :8082)       (Динамический рендер тайлов :8083)
         │                           │
         └─────────────┬─────────────┘
                       ▼
      [QGIS / Python (pystac-client, stackstac, rioxarray)]
```

### Ключевые возможности
* **HTTP Middleware & Auto-Signing:** Встроенная прослойка `reqwest-middleware` автоматически перехватывает запросы к закрытым хранилищам (например, `*.blob.core.windows.net`), запрашивает временные SAS-токены через API и кэширует их в памяти с учётом `msft:expiry`.
* **Двусторонняя консистентность (Self-Healing):** Перед загрузкой сервис проверяет наличие айтема как в базе `pgstac`, так и физически в бакете MinIO (`head_object`). Если запись в БД есть, а файл на S3 потерян — автоматически запускается восстановление.
* **Zero-Disk I/O транзит:** Бинарные ассеты не засоряют файловую систему, а стримятся из сети напрямую в бакеты MinIO через `aws-sdk-s3` с проверкой успешности HTTP-статусов (защита от сохранения XML-ошибок под видом растров).
* **Универсальность форматов:** Корректная обработка растровой оптики **COG (`.tif`)**, многомерных научных массивов **NetCDF (`.nc`)** и табличных данных **Parquet**.
* **Полный стек раздачи (Docker):** Готовая связка `pgstac` + `stac-fastapi` + `TiTiler` позволяет подключать локальный каталог напрямую в QGIS или аналитические Python-ноутбуки через стандартные библиотеки `pystac-client` и `stackstac`.

---

## 🛠 Технологический стек

* **Язык и рантайм:** Rust (2021/2024 edition), `tokio`
* **Сеть и Middleware:** `reqwest`, `reqwest-middleware`, `http`, `async-trait`, `chrono`
* **STAC Экосистема:** `stac`, `stac-client`, `pgstac` (паттерн Extension Traits поверх `tokio-postgres`)
* **Хранилище объектов:** `aws-sdk-s3`, `aws-config` (S3 Path-Style для работы с MinIO)
* **Инфраструктура (Docker):** `pgstac`, `pgsty/minio`, `stac-fastapi-pgstac`, `titiler-pgstac`

---

## 📂 Структура проекта

```text
stac_service/
├── Cargo.toml                        # Зависимости и фичи крейтов
├── docker-compose.yml                # Инфраструктура: MinIO, pgSTAC, STAC API, TiTiler
├── config.toml                       # Конфигурация S3/DB, провайдеров и коллекций
└── src/
    ├── main.rs                       # Точка входа и инициализация пайплайна
    ├── config.rs                     # Структуры и парсер TOML-конфигурации
    ├── middleware/
    │   └── microsoft_sas.rs          # HTTP Middleware для подписи и кэширования SAS-токенов
    └── sattelite_catalog_service.rs  # Ядро: пул клиентов, Self-Healing, транзит в S3, запись в БД
```

---

## 🚀 Быстрый старт

### 1. Развертывание инфраструктуры (Docker)
Поднимите хранилище MinIO, базу данных `pgstac`, REST-сервер `stac-api` и тайл-сервер `titiler`:

```bash
docker compose up -d
```

После запуска станут доступны:
* **MinIO API / Console:** `http://localhost:65226` / `http://localhost:65227`
* **PostgreSQL (pgSTAC):** `localhost:5439`
* **STAC FastAPI (Каталог и поиск):** `http://localhost:8082` (Swagger UI: `/api.html`)
* **TiTiler (Динамический рендер COG):** `http://localhost:8083`

### 2. Настройка конфигурации (`config.toml`)
Укажите параметры подключения к локальной инфраструктуре и список целевых коллекций:

```toml
[s3_settings]
endpoint = "http://localhost:65226"
bucket_name = "geo-data"
access_key = "minioadmin"
secret_key = "minioadmin"

[postgres_settings]
url = "postgresql://username:password@localhost:5439/postgis"

[sync_settings]
default_max_cloud_cover = 15.0
start_date = "2025-01-01T00:00:00Z"

# Провайдер 1: Microsoft Planetary Computer
[[providers]]
name = "microsoft"
api_url = "https://microsoft.com"

  [[providers.collections]]
  id = "landsat-c2-l2"
  description = "Landsat-8/9 COG (Оптика и тепловые каналы)"
  max_cloud_cover = 10.0
  bbox = [105.0, 55.0, 163.0, 74.0]

  [[providers.collections]]
  id = "sentinel-2-l2a"
  description = "Sentinel-2 COG (Высокое разрешение 10-20м)"
  max_cloud_cover = 15.0
  bbox = [105.0, 55.0, 163.0, 74.0]

# Провайдер 2: AWS Element84 Earth Search
[[providers]]
name = "element84"
api_url = "https://element84.com"

  [[providers.collections]]
  id = "sentinel-2-l2a"
  description = "Sentinel-2 COG через AWS"
  max_cloud_cover = 15.0
  bbox = [129.0, 61.0, 131.0, 63.0]
```

### 3. Сборка и запуск
Выполните сборку и запуск проекта в release-режиме:

```bash
cargo run --release
```

---

## 🗄️ Схема хранения и интеграция с клиентами

* **В MinIO:** Объекты раскладываются по иерархии провайдера и коллекции (с привязкой общих файлов вроде `preview.png` к `item_id` во избежание коллизий):
  `<provider>/<collection_id>/<filename>`
  *(Пример: `microsoft/landsat-c2-l2/LC09_L2SP_128021_20261103_02_T1_SR_B5.TIF`)*
* **В pgSTAC:** В таблицах `collections` и `items` регистрируются метаданные. Поля `assets.<key>.href` перезаписываются на внутренние пути `s3://geo-data/...` (а при необходимости дополняются расширением `alternate.http.href`).

### Подключение к QGIS и Python
1. **QGIS:** В панели *Браузер -> STAC -> New Connection* укажите `http://localhost:8082`. Для просмотра растров напрямую из приватного S3 используйте *TiTiler* (`http://localhost:8083`) в качестве XYZ-слоя.
2. **Python (`pystac-client`):**
   ```python
   import pystac_client

   catalog = pystac_client.Client.open("http://localhost:8082")
   search = catalog.search(collections=["landsat-c2-l2"], bbox=[115.8, -30.2, 116.5, -29.5])
   items = search.item_collection()
   ```

---

## 🔮 План развития (Roadmap)
* [x] **Идемпотентность и Self-Healing:** Двусторонняя проверка наличия метаданных в `pgstac` и бинарных файлов в MinIO (`head_object`).
* [x] **Авторизация SAS-токенов:** Внедрение `MicrosoftSasMiddleware` для прозрачной подписи URL и кэширования токенов Azure.
* [x] **Локальный STAC API и рендер:** Интеграция `stac-fastapi-pgstac` и `titiler-pgstac` в Docker Compose.
* [ ] **Белый список каналов (`target_assets`):** Фильтрация скачиваемых ассетов на уровне конфигурации для экономии дискового пространства и трафика (исключение служебных масок и метафайлов).
* [ ] **Ограничение конкурентности и Retry-политики:** Переход на `buffer_unordered` и `reqwest-retry` для защиты от троттлинга при работе на нестабильных каналах.
* [ ] **Режим `metadata_only`:** Поддержка индексации каталогов без скачивания растров для работы с внешними облаками через HTTP Range-запросы.
* [ ] **Структурированное логирование:** Замена `println!` на `tracing` с поддержкой уровней логирования.


