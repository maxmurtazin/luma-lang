# CAD-0 · результаты проверки

[Компоненты и исходники](README.md) · [Lock](../../luma-cad.lock.json)

## Стенды

| Target | Окружение | Проверенный объём |
| --- | --- | --- |
| T0 | Debian 13, Linux 6.18.57, x86-64, AMD EPYC 9V74, 5 видимых CPU, quota 4 CPU, cgroup RAM 16 GiB, overlayfs | Core и View собраны по lock на Rust 1.100.0. Прошли 185 тестов Core, 2 нативных теста View и 2 теста адаптера V2. 23 теста Luma, примеры и двуязычная документация проверены на Rust 1.96.0. |
| T1 | Chromium 156.0.8078.79, 800×600, WebGPU со SwiftShader, Node 24.22.3, Bun 1.4.4, wasm-bindgen 0.2.127 | Прошёл upstream E2E Core → View V2 с SQLite, HTTP и WASM. Проверены Part и Assembly, pick по полному пути, repin, restart и отсутствие model commits при управлении камерой и качеством. |

У сетки 14 982 вершины и 29 960 треугольников. В проверенных отсчётах
максимальное отклонение от существующего LIRVIEW1 — 0,812 нм. Два экземпляра
в сцене использовали один геометрический ресурс. [Отчёт Core → View V2](core-view-v2-t1.json)

Отдельный upstream suite `test:browser` дважды завершился ошибкой
`GPUBuffer.mapAsync` на Chromium 156. Этот suite проверяет standalone View
и дополнительные маршруты. На закреплённом Playwright Chromium 159.0.8154.6
этот suite дошёл до таймаута уточнения кадра B-rep V5. Приведённый результат
E2E относится к сценам
Core V2 с программным WebGPU. Проверка на физической GPU и квалификация
CAD-интерфейса Luma остаются `not_run`.

Bootstrap для существующего `luma check` в release build на T0 использовал
32 KiB исходника с 500 присваиваниями. После 5 разогревов сделано 30
измерений. Первый процесс 8,318 мс, p50 8,855 мс, p95 10,857 мс.
Пиковый RSS дочернего процесса 10 624 KiB, stdout 69 байт. Планирование,
коммит из Luma, отмена и полная performance matrix ожидают исполняемого CAD host.
Команда для повторения измерения запускается из корня Luma.

```sh
cargo +1.96.0 build --locked --release
python -B tools/cad/benchmark.py target/release/luma
```

## Матрица приёмки

| Проверка | Результат CAD-0 | Следующая проверка |
| --- | --- | --- |
| AT-003 | Pass · 18 проектных операций недоступны, в том числе с реализованным upstream backend или адаптером | Ответ CAD CLI/runtime после подключения registry |
| AT-004 | Pass · контрактные fixtures отклоняют различающиеся descriptors и строковые outputs | Проверка настоящих checker/runtime на общем host API |
| AT-005 | Pass · fixture CAD resolver возвращает `ModuleNameCollision`, legacy resolver выбирает sibling | Проверка в Luma resolver |
| AT-009 | Pass · decimal и LCAN fixtures проверяют NaN, infinity, u64 max, overflow и число выше 2⁵³ | Проверка admission и quantity constructors |
| AT-026 | Pass · fixture отклоняет независимо изменённый lock, provider fingerprint и generation | Проверка на prepared candidate в Core с CAS |
| AT-061 | Pass · registry/lock и двуязычный reference согласованы | Сверка с реальным CLI, completion и examples после включения binding |
| AT-077 | Pass · 11 LCAN vectors, в том числе Unicode, `-0`, PlanRef, sets, порядок DAG и комментарии | Сверка bytes при реализации нативного кодека |
| AT-078 | Pass · fixtures отклоняют неизвестную schema version, неизвестное поле и неизвестный operation major | Проверка отказа до Core mutation |
| AT-080 | Pass · ссылки, примеры wire и синхронные RU/EN tables проверены; 14 действующих примеров Luma исполнились | Runnable CAD examples после появления host API |

Статус Pass в таблице относится к перечисленному артефакту CAD-0.
Указанные в последнем столбце сквозные проверки имеют статус `not_run`.

## Воспроизведение

В чистых checkout установите Core и View на SHA и проверьте входные
файлы командой из [инструкции CAD-0](README.md). Для Core и View использован
Rust 1.100.0 с `wasm32-unknown-unknown`, для web build — Node 24 и
`wasm-bindgen 0.2.127`. Для gateway нужен Bun 1.4.4.

```sh
# В checkout View. CORE_CHECKOUT указывает на чистый checkout Core.
npm ci --ignore-scripts
cargo +1.100.0 test --locked -p living-view-scene -p living-view-contract -p living-view-renderer --lib
npm run build
node scripts/prepare-core-v2.mjs "$CORE_CHECKOUT"
npm --prefix "$CORE_CHECKOUT/frontend/e3a" ci --ignore-scripts
npm --prefix "$CORE_CHECKOUT/frontend/e3a" run build
bun install --cwd "$CORE_CHECKOUT/server/col1a" --frozen-lockfile

cargo +1.100.0 test --manifest-path "$CORE_CHECKOUT/Cargo.toml" --locked \
  -p hir-transaction -p living-cad-features -p living-sketch -p living-meshing
cargo +1.100.0 test --manifest-path "$CORE_CHECKOUT/Cargo.toml" --locked \
  -p living-cad-viewer --features living-view-v2 render_ --lib
cargo +1.100.0 build --manifest-path "$CORE_CHECKOUT/Cargo.toml" --locked \
  -p living-cad-viewer --features living-view-v2,embedded-ui --bin living-rust-service
WEBGPU_SOFTWARE=1 CHROMIUM_BIN=/usr/bin/chromium \
  node scripts/check-v2.mjs "$CORE_CHECKOUT" "$CORE_CHECKOUT/target/debug/living-rust-service"
```
