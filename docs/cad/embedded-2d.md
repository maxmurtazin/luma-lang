# CAD-2 · встроенный Living IR 2D

Встроенный адаптер хранит состояние `living-ir.2d.embedded/1` в
аутентифицированном реестре Core ProjectStore. Нативная ревизия Part и
ревизия встроенного 2D документа ведутся отдельно. Запись 2D
увеличивает ревизию проекта. Если пакет создаёт экструзию, в этой же
транзакции увеличивается нативная ревизия Part. Core фиксирует состояние и квитанцию одной
транзакцией SQLite. Адаптер использует временный решатель Living IR 2D в
памяти и передаёт кандидата в CAS Core.

Сборка включает запись по явному параметру `--ir2d`. Она использует
закреплённые версии поставщиков и оставляет исходные рабочие копии
провайдеров без изменений. Для включения 2D и доступа к B-rep из одной
базы соберите оба инструмента из изолированной сборки.

```sh
python -B tools/cad/prepare-brep-project.py "$CORE" "$BREP" "$BUILD" --ir2d "$IR2D"
cargo build --manifest-path "$BUILD/living-ir-core/Cargo.toml" \
  -p living-cad-viewer --features luma-cad,luma-embedded-2d --bin luma-cad-project
cargo build --manifest-path "$BUILD/native/Cargo.toml" --features luma-tasks
```

При отдельной сборке хоста можно применить
`tools/cad/prepare-core-project.py CLEAN_CORE --ir2d "$IR2D"` к отдельной
чистой копии Core. После регистрации 2D состояния используйте сборки с
`luma-embedded-2d` для чтения работающей базы. Сборка без расширения
отклоняет такой формат реестра.

## Команды владельца

Вызовы начинаются с `luma-cad-project DB PROJECT`. Передайте ключ владельца
как `LUMA_CAD_PROJECT_KEY`. Как и у остальных команд Core, `OPERATION`
задаётся 32 строчными hex-символами, а ожидания ревизий передаются
десятичными числами.

| Команда | Поведение |
| --- | --- |
| `2d-register PART PROJECT_REV OPERATION SEED.json` | Принимает валидный снимок Living IR 2D с `ownership_mode: embedded` и `document_id`, совпадающим с Part. |
| `2d-apply PART PROJECT_REV OPERATION BATCH.json` | Проверяет цель и ожидаемую ревизию 2D из нативного пакета команд, готовит кандидат и фиксирует его через Core. |
| `2d-luma PART PROJECT_REV REV_2D OPERATION BUNDLE.json` | Готовит типизированные команды эскиза, Draft и экструзии из Luma source bundle. |
| `2d-inspect PART` | Возвращает снимок и три ревизии. |
| `2d-solve-report PART SKETCH` | Возвращает отчёт принятого решения с DOF, рангом, остатками и ветвью. |

Хост 2D команд Luma поддерживает `sketch.create(sketch_id, plane_id,
profile_id)`, `sketch.point(sketch, id, x, y)`, `sketch.fix(sketch, id,
point, x, y)`, `sketch.line(sketch, id, from, to)`,
`sketch.circle(sketch, id, center, rim)`, `sketch.profile(sketch)` и
`draft.create(id)`. Для Draft доступны слой, точки, отрезок через
`draft.add(layer, id, from, to)` и перенос `draft.transform(entity, dx, dy)`.
Используются UUID, плоскость XY, координаты типа
Length и профиль `mechanical-si-v1`. Нативный решатель проверяет
замкнутость, пересечения, отверстия и ограничения до CAS.

`brep.extrude(feature_id, profile, height)` использует принятый в том же
пакете ProfileRef. Поддержаны один профиль XY с прямолинейной внешней
границей и круговыми отверстиями, положительная высота и пустой Part.
Нативный Sketch сохраняет ID исходных кривых и ревизию 2D снимка.
2D документ и авторская экструзия записываются одним Core CAS.
Экструзия закрепляет вычисленную геометрию Profile. Последующее
редактирование 2D сохраняет принятую ревизию экструзии доступной.
Для публикации тела после CAS используйте `luma-cad-brep source`,
`publish` и `measure`.

Поддерживаются локальные зависимости внутри снимка. Изменения,
требующие внешнего read set, истории undo/redo Living IR 2D или
подтверждения новой ветви решателя между вызовами, отклоняются.
Самостоятельный `lir2d` отклоняет сохранение embedded-снимка.

## Проверка

Для проверки сбоя транзакции добавьте `checkpoint-fault-injection`
в список features команды сборки Core. Запустите тест с обычным
самостоятельным `lir2d` из закреплённого checkout.

```sh
python -B tools/cad/test-embedded-2d.py "$PROJECT_CLI" "$IR2D" "$LIR2D" --brep-cli "$BREP_CLI"
cargo test --locked --manifest-path "$BUILD/native/Cargo.toml" --features luma-tasks
```

Сценарий 2D проверяет отказ до регистрации, запрет автономной записи,
отказ незамкнутого профиля, откат аварийной записи, повтор квитанции,
устаревшую ревизию, конфликт двух писателей, сохранение ID кривых
и инженерный объём опубликованной экструзии. Отдельная проверка B-rep
исполняет объявленные в Luma задачи над подлинной опубликованной
геометрией. [Задачи B-rep](brep-project.md)
