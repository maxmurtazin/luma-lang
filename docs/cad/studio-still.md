# CAD‑6 · Закреплённый кадр и видео Studio

Команда Luma `render.still` готовит один PNG по принятой ревизии
презентации, точному времени и камере. `prepare` показывает размеры,
оценку диска и хеши входов. `submit` передаёт тот же план штатной
очереди Studio. Очередь сохраняет пакет и возвращает ID задания.
Повтор заявки с теми же входами возвращает прежний ID, а изменение
входов с тем же ключом даёт `IDEMPOTENCY_CONFLICT`.

В проверенном сценарии Core построил сборку из двух экземпляров пластины
и восстановил её исторический снимок. `product-studio import-living-ir`
принял этот снимок, `render.still` поставил кадр в очередь и WebGL2
опубликовал PNG. Манифест `master.json` связывает ревизию, снимок, время,
формат цвета, версию Chromium и хеш PNG. Камера закреплена в `scene.json`,
хеш которого входит в план и манифест.

Профили проверены на Linux x86‑64 с Rust 1.100.0 и Node.js 24.
`preview-raster` использует Chromium, `final-pathtrace` — Blender Cycles
на CPU. Для финального рендера импортированной CAD-сборки Studio
требует закреплённую финальную сетку. Презентация, её правки и
перепривязка принадлежат хранилищу Studio.
[Ревизии провайдеров](../../luma-cad.lock.json) ·
[Контракт очереди](https://github.com/maxmurtazin/illusion-maker-product-studio/blob/938eafee7a6b1f0c2a9a00d98814a33b457dd33d/docs/S5-RASTER-EXPORT.md)

## Воспроизведение

`STUDIO` указывает на закреплённый checkout, `BUILD` на новую композицию.
Соберите обе программы в одном каталоге. Для системного Chromium
задайте `STUDIO_CHROMIUM`.

```sh
python3 -B tools/cad/prepare-studio-render.py "$STUDIO" "$BUILD"
cargo build --manifest-path "$BUILD/Cargo.toml" -p product-studio --bins
npm --prefix "$BUILD/apps/standalone" ci
ADAPTER="$BUILD/target/debug/luma-cad-studio"
NATIVE="$BUILD/target/debug/product-studio"
export STUDIO_CHROMIUM=/usr/bin/chromium
python3 -B tools/cad/test-studio-still.py "$BUILD" "$ADAPTER"
```

`STILL.json` задаёт ID, принятую ревизию, рациональное время и чётную
высоту от 64 до 1080 пикселей. Ширину определяет пропорция камеры. Для
контрольной презентации `init-mock` подходят следующие значения.

```json
{
  "presentation_id": "10000000-0000-4000-8000-000000000001",
  "revision": 1,
  "time": { "ticks": "1", "scale": 50 },
  "height": 144
}
```

```sh
COMMAND=examples/cad/studio-still-command.json
"$ADAPTER" prepare "$BUILD" "$PROJECT_DB" "$QUEUE" "$STILL" "$COMMAND"
"$ADAPTER" submit "$BUILD" "$PROJECT_DB" "$QUEUE" "$REQUEST" "$STILL" "$COMMAND"
node "$BUILD/apps/standalone/tools/render-queue.mjs" --root "$QUEUE" run
node "$BUILD/apps/standalone/tools/render-queue.mjs" --root "$QUEUE" status "$JOB"
```

Для Cycles добавьте в JSON `"profile": "final-pathtrace"`. Параметры
`samples` и `threads` задают от 1 до 128 выборок и от 1 до 16 потоков.
По умолчанию используются 16 выборок и 4 потока.

Для видео используйте `examples/cad/studio-video-command.json` и JSON с
ID, ревизией, высотой и профилем. `render.video` читает весь принятый
клип с его рациональным FPS. Studio публикует PNG-мастер RGB8 sRGB и
MP4 H.264 в ограниченном диапазоне BT.709. FFmpeg проверяет кадры и PTS.
В видео один поток изображения, аудиодорожка отсутствует.

```sh
"$ADAPTER" submit "$BUILD" "$PROJECT_DB" "$QUEUE" "$REQUEST" "$VIDEO" \
  examples/cad/studio-video-command.json
node "$BUILD/apps/standalone/tools/render-queue.mjs" --root "$QUEUE" run
node "$BUILD/apps/standalone/tools/render-queue.mjs" --root "$QUEUE" reencode "$JOB" --crf 23
python3 -B tools/cad/test-studio-still.py "$BUILD" "$ADAPTER" --extended
```

Команды `pause`, `resume` и `recover` используют тот же ID задания.
Studio проверяет сохранённые кадры при возобновлении. `reencode`
создаёт новый MP4 из проверенного PNG-мастера.

Для реальной сборки используйте автономный пробник Studio
`integration/probes/living_ir_studio_capture.rs` с Core из CAD lock.
Пробник добавляется через тестовый модуль `checkpoint.rs` в отдельной
копии Core. Запустите `studio_s6_capture_historical_assembly` с
`STUDIO_CAPTURE_DIR`, указывающим на новый каталог снимка.
[Рецепт автономного пробника](https://github.com/maxmurtazin/illusion-maker-product-studio/blob/938eafee7a6b1f0c2a9a00d98814a33b457dd33d/docs/S6-LIVING-IR-PREVIEW.md)

```sh
"$NATIVE" import-living-ir "$PROJECT_DB" "$STUDIO_CAPTURE_DIR"
python3 -B tools/cad/test-studio-still.py "$BUILD" "$ADAPTER" --capture "$STUDIO_CAPTURE_DIR"
```

## Правки презентации и перепривязка

`presentation-apply` принимает пакет Luma с `studio.apply` и нативный
конверт правок Studio. Операции оформления, камеры, света и действий
проверяются и принимаются одной SQLite-транзакцией Studio. Смещение
в презентации хранится отдельно от позы из CAD.

`rebind-preview` принимает снимок той же сборки и сохраняет план с
исходной ревизией, дайджестами снимков, изменениями и блокерами.
При неоднозначной или пропавшей связи можно явно передать
`DECISIONS.json` в последнем аргументе. Для принятия требуется конверт
команды Studio с ID презентации и `expected_revision` из плана.
Устаревший или изменённый план отклоняется. Повтор принятия с тем же
ключом возвращает исходную квитанцию. Поставленные задания сохраняют
пакеты от прежней ревизии.

```sh
"$ADAPTER" presentation-apply "$BUILD" "$PROJECT_DB" "$EDITS" \
  examples/cad/studio-apply-command.json
"$ADAPTER" rebind-preview "$BUILD" "$PROJECT_DB" "$UPDATED_CAPTURE" \
  examples/cad/studio-rebind-command.json "$PLAN"
"$ADAPTER" rebind-accept "$BUILD" "$PROJECT_DB" "$PLAN" "$HEADER"
python3 -B tools/cad/test-studio-still.py "$BUILD" "$ADAPTER" --extended \
  --capture "$STUDIO_CAPTURE_DIR" --updated-capture "$UPDATED_CAPTURE"
```
