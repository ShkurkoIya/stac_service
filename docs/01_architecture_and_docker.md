# 🐳 Инфраструктура Docker и сетевые нюансы

## Карта портов и сервисов

| Сервис | Контейнер | Порт хоста (Ubuntu) | Внутренний порт Docker | Назначение |
| :--- | :--- | :--- | :--- | :--- |
| `minio` | `geo_storage_s3` | `65226` (API)<br>`65227` (Console) | `9000`<br>`9001` | S3-совместимое объектное хранилище растров |
| `database` | `stac-db` | `5439` | `5432` | PostgreSQL + PostGIS + схема `pgstac` |
| `stac-api` | `stac-api-server` | `8082` | `8080` | REST STAC API (`stac-fastapi-pgstac`) |
| `titiler` | `stac-titiler` | `8083` | `8000` | Динамический тайл-сервер (`titiler-pgstac`) |

## Важные правила эксплуатации
1. **Имена хостов внутри Docker:** Контейнеры общаются по имени сервиса из `docker-compose.yml` (`database`, `minio`) и по **внутренним** портам (`5432`, `9000`). Внешние порты (`5439`, `65226`) используются только для подключения с хост-машины (Rust-бинарник, DataGrip, Python).
2. **Переменные окружения `stac-fastapi`:** Новые версии образа используют Pydantic v2 и требуют строгих переменных с префиксом `PG*` (`PGHOST`, `PGPORT`, `PGUSER`, `PGPASSWORD`, `PGDATABASE`). Старые переменные `DB_*` игнорируются.
3. **Перезапуск отдельного сервиса без остановки БД:**
   ```bash
   sudo docker compose up -d --no-deps --force-recreate stac-api
   ```

