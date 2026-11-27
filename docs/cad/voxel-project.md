# CAD-3 · Сфера Core и Living Voxel

Первый сценарий использует аналитическую сферу `GeometryPrimitive` из
`living-frep`. Адаптер реализует `FieldEvaluator` поставщика Living
Voxel. Ввод задаёт центр, радиус и регулярную сетку в метрах. Отсчёты
отрицательны внутри сферы. Поставщик маркирует их как оценки знакового
расстояния. Общая граница погрешности сеточного представления пока не
установлена.

Команда `sample` записывает новый LVX средствами Living Voxel. Запись
публикуется после полной подготовки файла. Существующий файл защищён от
перезаписи. Отчёт содержит точные ревизии поставщиков, SHA‑256 источника
и LVX и логический digest поля. При чтении проверяется SHA‑256 файла.
LVX сохраняет идентификатор источника и отпечатки бинарного файла и
Cargo.lock.

## Воспроизведение

`CORE` и `VOXEL` указывают на локальные checkout по
[CAD lock](../../luma-cad.lock.json). Скрипт собирает отдельное дерево из
этих коммитов. Проверено на Linux x86-64 с Rust 1.100.0.

```sh
python3 -B tools/cad/prepare-voxel-project.py "$CORE" "$VOXEL" "$BUILD"
cargo build --locked --manifest-path "$BUILD/native/Cargo.toml"
CLI="$BUILD/native/target/debug/luma-cad-voxel"
"$CLI" sample examples/cad/voxel-sphere.json sphere.lvx
"$CLI" inspect sphere.lvx "$LVX_SHA" 0 0 0
python3 -B tools/cad/test-voxel-project.py "$CLI"
```

В `LVX_SHA` передайте `digests.blob_digest` из ответа `sample`.
Сценарий проверяет 4 913 отсчётов, расстояние внутри и снаружи сферы,
источник после повторного открытия, отказ при неверном хеше и защиту
файла от перезаписи.

Этот маршрут семплирует аналитическую сферу, заданную входным файлом.

## Принятая сфера Part

Профиль `cad3-regular-sphere-v1` читает активную сферу редактора Core
из принятой нативной ревизии Part. Core и Living Voxel собираются в
отдельные бинарники со своими Cargo.lock. Это сохраняет точные версии
зависимостей обоих провайдеров.

```sh
cargo build --locked --manifest-path "$BUILD/project/Cargo.toml"
PROJECT_CLI="$BUILD/project/target/debug/luma-cad-voxel-project"
"$PROJECT_CLI" "$DB" "$PROJECT" source "$PART"
jq .grid examples/cad/voxel-sphere.json > grid.json
"$PROJECT_CLI" "$DB" "$PROJECT" submit-luma "$CLI" "$REQUEST" \
  "$PART" "$PART_REV" "$DEFINITION_HASH" grid.json examples/cad/voxel-sample-command.json
"$PROJECT_CLI" "$DB" "$PROJECT" work "$CLI"
"$PROJECT_CLI" "$DB" "$PROJECT" export "$JOB" part-sphere.lvx
```

Передайте ключ владельца через `LUMA_CAD_PROJECT_KEY`.
Команда `source` выдаёт `revision` и `definition_hash`. Клиент выбирает
UUID `REQUEST`. `submit-luma` проверяет одну типизированную команду
`voxel.sample` для заданных Part и сетки. ID `JOB` возвращается в поле
`job`. Провайдер запускается из частной копии выбранного бинарника,
чей отпечаток входит в задачу. Работа ограничена 30 секундами и
32 768 отсчётами для проектной сетки.

`receipt REQUEST` восстанавливает квитанцию после потери ответа.
`status JOB` показывает номер и состояние попытки. При отмене через
`cancel JOB` активный worker завершает дочерний процесс, а транзакция
публикации проверяет актуальность попытки. `retry JOB ATTEMPT` создаёт
новую попытку для отменённой или неудавшейся задачи. После сбоя посреди
вычисления можно отменить сохранённую попытку и повторить её.
Обновление Part не меняет исторический источник уже принятой задачи.

### Сечение, сетка и Living View

```sh
"$CLI" slice-xy part-sphere.lvx "$LVX_SHA" 0
"$CLI" surface part-sphere.lvx "$LVX_SHA" | jq -e .result > surface.json
python3 -B tools/cad/prepare-voxel-view.py "$VIEW" "$VIEW_BUILD"
cargo build --locked --manifest-path "$VIEW_BUILD/Cargo.toml" \
  -p living-view-core-adapter --bin pack-voxel
"$VIEW_BUILD/target/debug/pack-voxel" < surface.json > scene.json
VIEW_FIT=1 VIEW_EXPECTED_REVISION=1 WEBGPU_SOFTWARE=1 \
  node tools/cad/test-view-scene.mjs "$VIEW" scene.json
```

Сечение использует индекс Z сетки и сохраняет различие между
отсчётом и областью за её пределами. Сетка передаётся в метрах как
`ExternalAsset`. Она сохраняет ссылку на исходный LVX и хеш входного
артефакта. Отчёт Living Voxel содержит `Unavailable` для верхних
границ позиционной ошибки и расстояния Хаусдорфа. View вычисляет
только отображаемые нормали. Тест View требует собранную WASM-версию
закреплённого View и Chromium.

```sh
LUMA_VOXEL_PROVIDER="$CLI" cargo test --locked --manifest-path "$BUILD/project/Cargo.toml" \
  --features checkpoint-fault-injection --test project
```

Выпуск квалифицирует узловую регулярную сетку, исторические задания
для сферы и поверхность для View. Изменяемые блоки с footprint и
Undo, проектный paged GC, adaptive и lattice остаются за границами
этого выпуска.

[Манифест приёмки CAD-3](../../conformance/cad/cad3-regular-sphere-v1.json)
отдельно фиксирует проверенные, частично проверенные и открытые AT.
Для T1 проверен показ сетки в Chromium с программным WebGPU.
