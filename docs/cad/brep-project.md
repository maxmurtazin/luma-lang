# CAD-2 · профиль, экструзия и инженерные запросы

`luma-cad-brep` берёт принятую одностороннюю экструзию эскиза из Core
`ProjectStore`. `living-sketch` проверяет контуры, а закреплённый адаптер
B-rep переносит их направления и идентификаторы кривых. B-rep K1 строит
нормальную экструзию и публикует результат после проверки `L3K1Solid`.

Авторская модель остаётся в базе Core. Производные артефакты и квитанции
хранятся в отдельной SQLite-базе, привязанной к проекту. Перед публикацией
адаптер повторно сверяет ревизию и хеш детали под блокировкой писателя Core.
При следующем изменении детали прежняя публикация становится исторической.
Её проверенное тело и квитанция доступны по идентификатору операции.

## Сборка и проверка

Скрипт копирует закреплённые коммиты Core и B-rep в новый каталог сборки.
Пути и полные SHA находятся в lock. Проверен Linux x86-64 с Rust 1.100.0.
[Lock CAD](../../luma-cad.lock.json)

```sh
python3 -B tools/cad/prepare-brep-project.py "$CORE" "$BREP" "$BUILD"
mkdir -p "$EVIDENCE"
LUMA_CAD2_EVIDENCE="$EVIDENCE" cargo test --manifest-path "$BUILD/native/Cargo.toml"
cargo build --locked --manifest-path "$BUILD/native/Cargo.toml"
```

Интеграционный тест создаёт в Core прямоугольник 40×40 мм с круговым
отверстием радиусом 5 мм. После экструзии на 4 и 8 мм он проверяет
аналитический объём внутри интервала провайдера, стабильные ID пяти
исходных кривых, повтор операции, восстановление исторической квитанции и
отказ публикации по устаревшей ревизии. Тест также проверяет стабильность ID
эскиза и контуров, расстояния до тела и грани, внутреннее и внешнее сечения,
отказ для наклонного сечения и устаревшей ссылки на грань. Каждый вызов CLI
открывает базы в новом процессе. [Интеграционный тест](../../integrations/living-brep-core/cli_test.rs)

## CLI

Для существующего проекта задайте ключ владельца
`LUMA_CAD_PROJECT_KEY` из 64 hex-символов. `DERIVED` указывает на
отдельный файл SQLite. Команды после `DB PROJECT DERIVED` возвращают JSON
`luma.cad.brep-project/1`.

| Команда | Результат |
| --- | --- |
| `source PART FEATURE` | Возвращает текущую ревизию, хеш нативного определения и ID эскиза. |
| `sketch-profile PART FEATURE` | Возвращает канонические Sketch и Profile с точным pin, ID кривых, направлениями рёбер и результатом проверки формы. |
| `publish PART FEATURE REVISION DIGEST OPERATION_UUID` | Публикует B-rep для точного pin. Результат `published` содержит квитанцию. |
| `latest PART FEATURE` | Показывает последнюю публикацию и её актуальность относительно Core. |
| `measure OPERATION_UUID` | Проверяет опубликованный артефакт и возвращает интервал объёма в м³, исходные контуры, отчёт и provenance. |
| `view OPERATION_UUID` | Тесселирует проверенный артефакт и возвращает пакет для `pack-publication` в `result`. |
| `sheet-publish PART LIR2D INPUT SHEET POLICY OUTPUT [--font FONT]` | Выпускает чертёж аналитической пластины из принятой ревизии Part. |
| `distance OPERATION_UUID GEOMETRY_SHA solid\|face:N X Y Z` | Возвращает интервал расстояния до границы в метрах и признак возможных нескольких минимумов. Координаты заданы в локальной системе тела. |
| `section OPERATION_UUID GEOMETRY_SHA FRAME_JSON` | Возвращает аналитическое сечение нормальной экструзии плоскостью, параллельной её основанию. |

`GEOMETRY_SHA` — 64 строчные hex-цифры. `face:N` использует ordinal внутри
артефакта с этим хешем. Для другого хеша CLI возвращает
`StaleTopologyRef`. Для намеренного доступа к историческому телу укажите
ID и хеш его публикации. `FRAME_JSON` содержит числовые векторы `origin_m`,
`x_axis`, `y_axis`, `z_axis`. Тесселяция передаёт ordinal каждой грани и
границы своей погрешности. Выбор в View сообщает точность
`brep_display_tessellation`. Для инженерного расстояния используйте
`distance`, для объёма — `measure`.

В Core research sketch хранится вычисленная геометрия. DTO передаёт
сегменты, окружности и дуги с родными ID. Для профиля доступны один
внешний контур и отверстия первого уровня. Отдельный отчёт решателя
ограничений доступен для [самостоятельного Living IR 2D](standalone-2d.md).

## Проверка в Living View

Пакет `brep-publication.json` из теста содержит ровно `result` команды
`view`. Для ручного вызова извлеките это поле из оболочки команды. Сборка
пакера добавляет точный SHA производителя B-rep из lock к допускаемым
коммитам View. Пакет сохраняет фактические SHA производителя и геометрии.

```sh
python -B tools/cad/prepare-brep-view.py "$VIEW" "$VIEW_BUILD"
cargo run --locked --manifest-path "$VIEW_BUILD/Cargo.toml" \
  -p living-view-core-adapter --bin pack-publication \
  < "$EVIDENCE/brep-publication.json" > "$EVIDENCE/scene.json"
(cd "$VIEW" && npm ci --ignore-scripts && node scripts/build-web.mjs && npx playwright install chromium)
VIEW_BREP=1 VIEW_EXPECTED_REVISION=1 WEBGPU_SOFTWARE=1 \
  node tools/cad/test-view-scene.mjs "$VIEW" "$EVIDENCE/scene.json"
python -B tools/cad/validate.py --sketch-profile "$EVIDENCE/sketch-profile.json"
```

Скрипт View требует Node.js 24, Rust с target `wasm32-unknown-unknown`
и версию `wasm-bindgen`, закреплённую в lock View. Chromium 159 со
SwiftShader подтвердил рендеринг и выбор грани B-rep. Проверка WebGPU
на физическом GPU остаётся отдельной.

## Отдельный профиль M7

На Linux x86-64 проверен закреплённый `brep-planar-boolean` для `Union`,
`Intersection` и `Difference`. Операнды имеют одну жёсткую систему
координат и одинаковую знаковую высоту. Результат содержит один связный
материал с отверстиями первого уровня. Для пространственных операндов,
касания и несвязного результата предусмотрены отказы. Выпуск M7
использует собственные versioned selectors, численный профиль и SHA
сборки. Git-коммит в `$BREP` должен совпадать с [lock](../../luma-cad.lock.json).

```sh
M7="$BREP/experimental/rectangle-union"
cargo test --locked --release --manifest-path "$M7/Cargo.toml"
cargo build --locked --release --manifest-path "$M7/Cargo.toml"
cargo build --locked --release --manifest-path "$BREP/Cargo.toml" -p brep-cli
python -B "$M7/fixtures/qualify_m7.py" \
  --boolean "$M7/target/release/brep-planar-boolean" \
  --core-cli "$BREP/target/release/brep-cli" > "$EVIDENCE/m7-linux.json"
```

Корпус прошёл 2200 сценариев, восемь проверок отклонения и повторную
загрузку выбранных нативных тел. M7 квалифицирован отдельно от
проектной публикации K1. Luma host binding для M7 остаётся запланированным.

## Задачи B-rep из Luma

В сборке с `--features luma-tasks` команда
`luma-cad-brep DB PROJECT DERIVED luma-tasks OPERATION_UUID GEOMETRY_SHA BUNDLE.json`
берёт геометрию из проверенной публикации. Luma объявляет задачи, после
успешной подготовки CLI выполняет их закреплённым провайдером. Поддержаны
одна задача тесселяции, параллельные сечения XY, интервал объёма и
расстояние до границы тела. До восьми задач выполняются синхронно
по отдельному вызову. Перед подготовкой проверяется хеш B-rep.

```luma
use units
use brep
use cad
body = brep.published_body()
options = brep.task_options()
cad.mesh(body, options)
cad.section_task(body, brep.xy_section(units.mm(2)), options)
cad.query_task(body, brep.volume(), options)
```

## Объём приёмки

Профиль CAD-2 включает встроенный Core CAS, Luma-команды для Sketch
и Draft, нативную экструзию из ProfileRef и отдельный исполнитель
инженерных задач. Расширение на режим recipe, внешние зависимости
2D, явное принятие новой ветви, долговечную очередь B-rep задач и
Luma binding для отдельного профиля M7 ещё открыто. [Встроенный 2D](embedded-2d.md)
[Критерии CAD-2](../../spec/cad/Luma-CAD-Development-Specification-v1.0-RU.md#sec-32)
