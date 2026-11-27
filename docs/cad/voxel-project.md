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

Профиль регулярной сферы квалифицирует узловую сетку, исторические
задания для сферы и поверхность для View. Правки резидентных блоков
и paged GC квалифицированы в проектном профиле ниже.

[Манифест приёмки CAD-3](../../conformance/cad/cad3-regular-sphere-v1.json)
отдельно фиксирует проверенные, частично проверенные и открытые AT.
Для T1 проверен показ сетки в Chromium с программным WebGPU.

## Профиль проектных объёмов

Профиль `cad3-voxel-project-v1` добавляет проектные Bool-маски,
страничное хранилище и операции над опубликованными полями. Для этих
команд нужен нативный `voxel-cli` из закреплённого Living Voxel. Выше
для семплирования используется отдельный `luma-cad-voxel`.

```sh
cargo build --locked --manifest-path "$BUILD/voxel/Cargo.toml" -p voxel-cli
VOXEL_CLI="$BUILD/voxel/target/debug/voxel-cli"
LUMA_VOXEL_PROVIDER="$CLI" LUMA_VOXEL_CLI="$VOXEL_CLI" \
  cargo test --locked --manifest-path "$BUILD/project/Cargo.toml" \
  --features checkpoint-fault-injection --test project
```

Передавайте абсолютный путь к доверенному локальному `voxel-cli`.
`MASK_DB` указывает на отдельную SQLite-базу публикаций проекта.
`OBJECTS` указывает на выделенный каталог страничного хранилища.
Команда `seed` привязывает готовый LVX с резидентным Bool-каналом и
проверенным SHA‑256 к принятой ревизии Part. Источником правок служит
конкретная публикация. Все идентификаторы публикаций задаются как UUID.
`seed` принимает ожидаемое поколение слота, для нового слота передайте 0.

```sh
"$PROJECT_CLI" "$DB" "$PROJECT" mask "$MASK_DB" "$VOXEL_CLI" \
  seed "$BASE" "$PART" "$PART_REV" "$DEFINITION_HASH" mask 0 mask.lvx "$MASK_SHA"
"$PROJECT_CLI" "$DB" "$PROJECT" mask "$MASK_DB" "$VOXEL_CLI" patch "$EDIT" "$BASE" patch.json
"$PROJECT_CLI" "$DB" "$PROJECT" mask "$MASK_DB" "$VOXEL_CLI" undo "$UNDO" "$EDIT"
"$PROJECT_CLI" "$DB" "$PROJECT" mask "$MASK_DB" "$VOXEL_CLI" export "$UNDO" undo.lvx
```

Правка `replace` записывает до 64 полных блоков. `data: null` удаляет
блок, после чего применяется фон канала. `copy_face` читает соседний по
грани блок исходной публикации. CLI выбирает footprint в зависимости от
операции. Запись в тот же блок или область чтения отклоняется,
включая случай возврата прежних байтов. Подготовленная правка проверяет
поколение слота при публикации. Повтор завершённой команды возвращает
квитанцию. `status UUID` читает квитанцию, `head PART SLOT` читает
текущую публикацию в слоте.

```json
{"operation":"copy_face","channel":"mask","from":[1,0,0],"to":[0,0,0]}
```

`mask … boolean or LEFT RIGHT CHANNEL` обрабатывает две исторические
маски, сохраняя их привязки. Поддерживаются также `and` и `diff`.
`field-csg VOXEL_CLI difference LEFT_JOB RIGHT_JOB implicit` применяет
min/max CSG к двум LVX-задачам семплирования. Также доступны `union` и
`intersection`. Результат содержит поле `ImplicitScalar` со статусом
`NotDistance` и ссылки на обе публикации. Запрос точного расстояния
отклоняется до запуска Voxel. Оба маршрута возвращают результат по stdout.

```sh
printf '%s\n' '{"max_objects":256,"max_object_bytes":65536,"max_total_bytes":2097152}' > limits.json
printf '%s\n' '[[0,0,0],[8,0,0]]' > indices.json
"$PROJECT_CLI" "$DB" "$PROJECT" paged "$MASK_DB" "$VOXEL_CLI" "$OBJECTS" pack "$UNDO" limits.json
"$PROJECT_CLI" "$DB" "$PROJECT" paged "$MASK_DB" "$VOXEL_CLI" "$OBJECTS" \
  query "$UNDO" mask indices.json limits.json
"$PROJECT_CLI" "$DB" "$PROJECT" paged "$MASK_DB" "$VOXEL_CLI" "$OBJECTS" gc limits.json
"$PROJECT_CLI" "$DB" "$PROJECT" paged "$MASK_DB" "$VOXEL_CLI" "$OBJECTS" \
  gc limits.json "$PLAN_DIGEST"
```

Публикация `pack` создаёт неизменяемую ссылку для конкретного
исторического LVX. `query` различает неизвестный отсчёт, незагруженный
блок и выход за сетку. В `PLAN_DIGEST` передайте `plan_digest`. GC под
блокировкой перепроверяет план и помещает недостижимые объекты в
карантин. Тест на Linux проверяет сохранность снимков и блоков при
исчерпании бюджета и после выполнения GC.

Для проверки View добавьте `VIEW_RECOVERY=1` к описанному выше запуску
со сценой сферы. Проверка отклоняет устаревший delta и неизвестный узел,
восстанавливает прежнюю сцену после потери WebGPU и проверяет
`unresolved` на закреплённой сцене полей Living View.

Общий сценарий пластины проверяет размеры и положение отверстий в
обеих ревизиях, перемычку 7 мм и стабильный выход Part. Показ принятой
ревизии входит в [квалификацию Core и View](core-project.md).

```sh
cargo test --manifest-path "$BUILD/core/Cargo.toml" -p living-cad-viewer \
  --features luma-cad --lib luma_cad1_recipe_command_cas_retry_restart
```

[Манифест проектного профиля](../../conformance/cad/cad3-voxel-project-v1.json)
отражает область приёмки. В этой конфигурации семплирование авторской
геометрии поддерживает сферу. Авторские lattice, adaptive-сетки и
точный экспорт B-rep требуют отдельно квалифицированных профилей.
