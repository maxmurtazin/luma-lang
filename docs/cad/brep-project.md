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

<a id="living-view"></a>

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
проектной публикации K1.

### Запуск M7 из Luma

Соберите `luma-cad-brep` с `--features luma-tasks`.
Команда `luma-m7` берёт две существующие публикации проекта.
Для каждой передаются UUID операции и хеш геометрии из `view`.
`M7_PROVIDER` указывает на доверенный `brep-planar-boolean`, собранный
из закреплённого в CAD lock B-rep. CLI сверяет контракт, целевую
платформу и SHA-256 исполняемого файла. Ключ проекта остаётся у CLI.

```sh
luma-cad-brep "$DB" "$PROJECT" "$DERIVED" \
  luma-m7 "$M7_PROVIDER" "$LEFT_OP" "$LEFT_SHA" "$RIGHT_OP" "$RIGHT_SHA" "$BUNDLE"
```

Командный bundle объявляет одну операцию. Для пересечения и
разности используются соответственно `brep.planar_intersection` и
`brep.planar_difference`. Разность вычитает второй аргумент из первого.

```luma
use brep
left = brep.left_operand()
right = brep.right_operand()
result = brep.planar_union("result", left, right)
```

Операнды должны иметь одинаковую систему координат и знаковую высоту.
Два диска и пара «диск + прямолинейный многоугольник без отверстий»
передаются в соответствующие режимы M7. Для пары явных профилей
допускаются отрезки и дуги. Цельная окружность в контуре отверстия
отклоняется провайдером.

CLI возвращает результат и нативные байты B-rep в JSON после проверки
хеша запроса, повторной загрузки и валидации L3. В ответе остаются
порядок операндов, их UUID и хеши геометрии, версия контракта и
fingerprint сборки. Вычисление выдаёт отдельный артефакт. Core и база
публикаций сохраняют прежнее состояние.

```sh
LUMA_CAD2_M7="$M7_PROVIDER" cargo test --locked --manifest-path "$BUILD/native/Cargo.toml" \
  --features luma-tasks --test project evaluates_luma_m7_using_two_pinned_native_publications -- --ignored
```

## Задачи B-rep из Luma

В сборке с `--features luma-tasks` команда
`luma-cad-brep DB PROJECT DERIVED luma-tasks OPERATION_UUID GEOMETRY_SHA BUNDLE.json`
берёт геометрию из проверенной публикации. Luma объявляет задачи, после
успешной подготовки CLI выполняет их закреплённым провайдером. Поддержаны
одна задача тесселяции, параллельные сечения XY, интервал объёма и
расстояние до границы тела. Один пакет содержит до восьми задач.
Перед подготовкой проверяется хеш B-rep.

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

### Долговечная очередь

Сборка с `--features luma-tasks` позволяет поставить тот же пакет в
очередь Core. `REQUEST_UUID` выбирает клиент. Повтор с тем же UUID и
тем же планом возвращает прежний ID задания. Если план изменился,
команда возвращает `RequestIdReuse`.

| Команда | Результат |
| --- | --- |
| `jobs-submit REQUEST_UUID OPERATION_UUID GEOMETRY_SHA BUNDLE.json` | Сохраняет подготовленный пакет и возвращает числовой ID `job`. |
| `jobs-receipt REQUEST_UUID` | Восстанавливает квитанцию после потери ответа без файла Luma. |
| `jobs-work` | Обрабатывает очередной пакет для этой версии провайдера. Возвращает квитанцию или `null`, если подходящих заданий нет. |
| `jobs-status JOB` | Показывает состояние, номер попытки и закреплённую ревизию Part. |
| `jobs-result JOB` | Возвращает сохранённый отчёт и манифест с SHA-256. |
| `jobs-cancel JOB` | Отменяет активное задание. Опубликованный результат остаётся доступен. |
| `jobs-retry JOB ATTEMPT` | Возвращает отменённое или неудавшееся задание в очередь при совпадении номера последней попытки. |

Очередь и отчёт хранятся в базе Core. Пакет закрепляет ID публикации,
хеши её геометрии и provenance, ревизию и хеш Part, а также версию
провайдера. Worker использует исходную базу публикаций `DERIVED`.
Изменение Part и новая публикация не меняют ранее принятое задание.
Файл Luma требуется при постановке. Worker исполняет сохранённый план.

После остановки worker посреди вычисления статус показывает `running`
и номер попытки. Чтобы запустить новую попытку, вызовите `jobs-cancel`,
затем `jobs-retry` с этим номером. Результат и манифест публикуются
одной транзакцией. Отменённая попытка не может опубликовать результат.
Смена версии провайдера требует сохранить предыдущую сборку для
обработки её незавершённых заданий.

```sh
cargo test --locked --manifest-path "$BUILD/native/Cargo.toml" \
  --features luma-tasks,checkpoint-fault-injection --test project \
  publishes_pinned_normal_extrusion_and_measures_historical_brep
```

## Объём приёмки

Профиль CAD-2 включает встроенный Core CAS, Luma-команды для Sketch
и Draft, рецепт Sketch/B-rep с добавлением объектов и отсоединением,
нативную экструзию из ProfileRef, очередь инженерных задач и
зависимости 2D от параметров и сущностей Part того же проекта.
[Встроенный 2D](embedded-2d.md)
[Критерии CAD-2](../../spec/cad/Luma-CAD-Development-Specification-v1.0-RU.md#sec-32)

| Критерий | Проверяемый результат |
| --- | --- |
| AT-037 | После повторной публикации операция с ordinal и старым хешем возвращает `StaleTopologyRef`. [Тест B-rep](../../integrations/living-brep-core/cli_test.rs) |
| AT-044, AT-049 | Нативная односторонняя экструзия публикует проверенное тело с интервалами и ID источника. Наклонное сечение и неподдержанные входы M7 получают отказ провайдера. [Тесты B-rep и M7](../../integrations/living-brep-core/cli_test.rs) |
| AT-054 | Проверяются отчёт решателя, явное принятие смены ветви, отказ для незамкнутого контура и добавление отверстия с сохранением существующих кривых. [Тест встроенного 2D](../../tools/cad/test-embedded-2d.py) |
| AT-057 | Встроенная команда требует зарегистрированный Part и принимает изменения через Core CAS. Сценарии включают аварийный откат, конфликт и повтор квитанции. [Тест встроенного 2D](../../tools/cad/test-embedded-2d.py) |
| AT-058 | Проверяются A4/A3, масштаб, связанные размеры в миллиметрах, PDF/SVG/DXF и отказы при нарушении правил выпуска. [Проверка чертежей](standalone-2d.md) |
| AT-068 | View выбирает грань по тесселяции, а нативный B-rep возвращает инженерные интервалы расстояния и объёма. [Проверка в Living View](#living-view) |

Рецепт рассчитан на одну плоскость XY, один замкнутый профиль и
положительную нормальную экструзию. Удаление и замена объектов
активного рецепта, межпроектные ссылки и запись Core-проекций внутрь
2D снимка остаются за его пределами.
