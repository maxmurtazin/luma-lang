# CAD-2 · самостоятельный Living IR 2D

Living IR 2D владеет своим `.lir2d` документом, ограничениями, параметрами
и принятым решением. Адаптер читает один снимок и экспортирует
канонические Sketch/Profile для проверенного `solid_profile` на XY.
Экспорт сохраняет ID сегментов и окружностей, их авторское направление,
ориентацию контуров и ссылку на ревизию источника. `project_id` в целевом
pin совпадает с `document_id`, потому что сам снимок задаёт только
идентичность документа. `source_container_sha256` привязывает ответ к
точным байтам файла. [Схема Sketch/Profile](../../schema/cad/sketch-profile-v1.json)

Связанные с единственным сегментом ограничения horizontal и vertical
переносятся в DTO. Полный `solver_report` содержит нативные ограничения,
степени свободы, ранг, остаточные ошибки, политику продолжения ветви,
seed и токен ветви, если он задан. Редактирование, принятие новой ветви и
диагностика при конфликте проходят через команды `lir2d preview` и
`lir2d apply`. Адаптер сохраняет диагностику провайдера в `Living2dError`.

Embedded-снимки отклоняются с `UnsupportedEmbedded2d`. Для записи
потребуются зарегистрированное typed state и подготовленный delta с
единым commit в Core CAS. Нативный `lir2d apply` также отклоняет запись
embedded-снимка. [Спецификация доменов 2D](../../spec/cad/Luma-CAD-Development-Specification-v1.0-RU.md#sec-19)

## Воспроизведение

Соберите `lir2d` из ревизии `ir-2d` в lock. `FONT` указывает на
TrueType-шрифт с нужными глифами. `TEST_OUT` должен быть новым каталогом.
[Lock CAD](../../luma-cad.lock.json)

```sh
cargo build --locked --manifest-path "$IR2D/Cargo.toml" --bin lir2d
LIR2D="$IR2D/target/debug/lir2d"
python -B tools/cad/read-2d-profile.py "$LIR2D" "$DOCUMENT" "$PROFILE_UUID"
python -B tools/cad/test-standalone-2d.py "$IR2D" "$LIR2D" "$FONT" "$TEST_OUT"
python -B tools/cad/validate.py \
  --sketch-profile "$TEST_OUT/sketch-profile-100.json" \
  --sketch-profile "$TEST_OUT/sketch-profile-120.json"
cargo test --locked --manifest-path "$IR2D/Cargo.toml" -p lir2d-cli --test snapshot_cli
cargo test --locked --manifest-path "$IR2D/Cargo.toml" -p lir2d-runtime --test transactions
```

Тест проверяет изменение ширины пластины со 100 до 120 мм со стабильными
ID кривых и контуров, нулём степеней свободы и перемычкой 7 мм.
Проверены A4 и A3, выпуск SVG/PDF в масштабе 1:1 и 1:2, физический
`MediaBox`, значения связанных размеров в мм, pin исходного Draft и отказ
при отсутствующей опоре. DXF содержит замкнутый внешний контур и два
круговых отверстия в модельных миллиметрах. Тесты закреплённого провайдера
дополнительно проверяют конфликт constraints, явное принятие ветви,
открытые и касающиеся контуры, связь DXF с моделью при смене масштаба
чертежа и отказ выпуска при несовпадении источника Core.

## Ассоциативная пластина Core

С параметром `--core "$CORE"` тест получает четыре аналитические
проекции из нативного `CanonicalFacade::drawing_plate_source`, сверяет
их с `DefinitionHash` и проверяет выпуск для просмотра. Для выпуска по
manufacturing policy провайдер требует подтверждённую границу
погрешности. Тест проверяет отказ `projection.quality_unproven` и отказ
`projection.core_source_ambiguous`, если две опоры указывают на одну
операцию. В обоих случаях файл выпуска отсутствует.

```sh
python -B tools/cad/test-standalone-2d.py "$IR2D" "$LIR2D" "$FONT" "$CORE_TEST_OUT" --core "$CORE"
bash "$IR2D/tools/core/read-parameter.sh" "$CORE" --test-hosted "$LIR2D"
```

Тесты провайдера проверяют выпуск под блокировкой `DefinitionStore`,
конфликт при изменении Core между подготовкой и выпуском и
воспроизведение исторического SVG.

### Выпуск из ProjectStore

`sheet-publish` получает геометрию всех проекций листа из одной принятой
ревизии выбранного Part. Команда использует нативный источник
`CreatePlate` для осевой пластины. Поддерживаются несколько таких
источников в пределах Part и ассоциативные размеры по ID опор.
Core-привязки параметров и проекции из других Part отклоняются.

Рендеринг выполняет доверенный `lir2d` из [CAD lock](../../luma-cad.lock.json).
Ему передаётся снимок листа и манифест геометрии. Ключ владельца остаётся
у команды Core. После проверки SHA‑256 файла команда коротко берёт
блокировку писателя ProjectStore, сверяет ревизию и хеш Part и публикует
файл через жёсткую ссылку без перезаписи. Изменение Part во время
рендеринга возвращает `Conflict`. Каталог назначения должен поддерживать
жёсткие ссылки.

```sh
"$BUILD/native/target/debug/luma-cad-brep" "$DB" "$PROJECT" "$DERIVED" \
  sheet-publish "$PART" "$LIR2D" "$DOCUMENT" "$SHEET_UUID" review "$OUTPUT" --font "$FONT"
```

`OUTPUT` оканчивается на `.svg` или `.pdf`. Политика `review` допускает
аналитические проекции с неизвестной границей погрешности. Политика
`manufacturing` отклоняет такие проекции. Для команды нужен ключ
`LUMA_CAD_PROJECT_KEY`. Сборка добавляет API чтения в изолированную
копию закреплённого Core по инструкции. [Сборка `luma-cad-brep`](brep-project.md)

```sh
LUMA_CAD2_IR2D="$IR2D" LUMA_CAD2_LIR2D="$LIR2D" LUMA_CAD2_DRAWING_FONT="$FONT" \
  cargo test --locked --manifest-path "$BUILD/native/Cargo.toml" --test project \
  publishes_associative_sheet_only_at_current_part_revision -- --ignored
```
