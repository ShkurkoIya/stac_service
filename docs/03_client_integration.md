# 🛰️ Интеграция с QGIS, Python и REST API

## 1. Подключение к QGIS
1. **Поиск по каталогу:** В панели *Браузер (Browser)* ➡️ *STAC* ➡️ *New Connection* указать `http://localhost:8082`.
   * *Совет:* При поиске через `Ctrl + L` (вкладка STAC) снимайте галочку *Filter by map extent*, если карта сфокусирована на другом регионе.
2. **Визуализация "черных" 16-битных растров:** Спутниковые каналы (`UInt16`) имеют узкий динамический диапазон. Для проявления рельефа: *ПКМ по слою ➡️ Свойства ➡️ Стиль ➡️ Отсечение по кумулятивному счету (2% - 98%)*.
3. **Просмотр растров напрямую из приватного S3 (через TiTiler):** Добавить слой *XYZ Tiles* с URL:
   `http://localhost:8083/collections/{collection}/items/{item_id}/tiles/WebMercatorQuad/{z}/{x}/{y}?assets=red,green,blue&rescale=0,3000`

## 2. Аналитика в Python (`pystac-client` + `rasterio`)
```python
import pystac_client
import rasterio
from rasterio.windows import Window

# 1. Поиск снимков через локальный STAC API
catalog = pystac_client.Client.open("http://localhost:8082")
search = catalog.search(
    collections=["landsat-c2-l2"],
    bbox=[115.8, -30.2, 116.5, -29.5],
    query={"eo:cloud_cover": {"lt": 10}}
)
items = search.item_collection()

# 2. Чтение матрицы пикселей через HTTP Range-запрос без скачивания всего файла
http_url = items[0].assets["SR_B5"].extra_fields["alternate"]["http"]["href"]
with rasterio.open(http_url) as src:
    window = Window(src.width // 2, src.height // 2, 100, 100)
    matrix = src.read(1, window=window)
    print(matrix.shape)
```

## 3. Тестовый CQL2-запрос через `curl`
```bash
curl -s -X POST http://127.0.0 \
  -H "Content-Type: application/json" \
  -d '{
    "collections": ["landsat-c2-l2"],
    "bbox": [115.8, -30.2, 116.5, -29.5],
    "limit": 2,
    "filter-lang": "cql2-json",
    "filter": {"op": "<", "args": [{"property": "eo:cloud_cover"}, 10]}
  }' | python3 -m json.tool
```

