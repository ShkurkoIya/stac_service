# 🗄️ SQL-шпаргалка для управления pgSTAC

Подключение в DataGrip / DBeaver: `localhost:5439`, БД `postgis`, схема `pgstac`.

## 1. Мониторинг наполнения базы
```sql
-- Список зарегистрированных коллекций
SELECT id, content->>'description' AS description 
FROM pgstac.collections;

-- Количество скачанных снимков по каждой коллекции
SELECT collection, count(*) AS items_count 
FROM pgstac.items 
GROUP BY collection
ORDER BY items_count DESC;
```

## 2. Инспекция метаданных и ссылок на файлы
```sql
-- Проверить пути s3:// и каналы конкретного снимка
SELECT id, datetime, content->'assets' AS assets 
FROM pgstac.items 
WHERE collection = 'landsat-c2-l2' 
LIMIT 5;
```

## 3. Патч для добавления альтернативных HTTP-ссылок (STAC Alternate Extension)
Позволяет сохранить основной путь `s3://`, но добавить `http://` для клиентов без S3-авторизации (требует политики `public/readonly` на бакет в MinIO):
```sql
UPDATE pgstac.items
SET content = jsonb_set(
    content,
    '{assets}',
    (
        SELECT jsonb_object_agg(
            key,
            value || jsonb_build_object(
                'alternate',
                jsonb_build_object(
                    'http',
                    jsonb_build_object(
                        'href',
                        replace(value->>'href', 's3://geo-data/', 'http://localhost:65226/geo-data/')
                    )
                )
            )
        )
        FROM jsonb_each(content->'assets')
    )
)
WHERE content ? 'assets';
```

## 4. Полная очистка каталога перед новым прогоном
```sql
-- Удалить только снимки (коллекции останутся)
TRUNCATE pgstac.items CASCADE;

-- Удалить всё, включая коллекции
TRUNCATE pgstac.collections, pgstac.items CASCADE;
```

